//! Recovering the secret a user's own files are wrapped with. A phone
//! keeps, for each user, a blob that only two things together open: a key
//! its secure world holds, and a secret that follows from what the user
//! knows. What comes out is the synthetic password, and every key that
//! belongs to the user follows from that.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{bail, ensure, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Each of these names one use of the hashing, so the same bytes hashed
/// for two purposes never agree.
const SECDISCARDABLE: &[u8] = b"secdiscardable-transform";
const GATEKEEPER: &[u8] = b"user-gk-authentication";
const SECRET_GATEKEEPER: &[u8] = b"sp-gk-authentication";
const PROTECTOR_SECRET: &[u8] = b"application-id";
const FILE_KEY: &[u8] = b"fbe-key";
const WEAVER_KEY: &[u8] = b"weaver-key";
const WEAVER_VALUE: &[u8] = b"weaver-pwd";
const CONTEXT: &[u8] = b"android-synthetic-password-personalization-context";

/// The only layout a phone writes the number of a weaver slot in.
const WEAVER_SLOT_LAYOUT: u8 = 1;

/// What stands in for a credential on a user who has set none.
const NO_CREDENTIAL: &[u8] = b"default-password";

/// How long the stretched credential is, whether it was stretched or
/// stood in for.
const STRETCHED_BYTES: usize = 32;

const NONCE_BYTES: usize = 12;
const KEY_BYTES: usize = 32;

/// The layout the blob is written in. Only the newest is read here: the
/// two before it derive their subkeys another way, and a phone that still
/// has one has not been through an update in years.
const LATEST: u8 = 3;

/// The blob belongs to what the user knows, rather than to a token some
/// other part of the phone holds.
const BY_CREDENTIAL: u8 = 0;

/// What a user's secret is kept behind.
pub struct Protector {
    /// What the phone calls it, which is also what the key held for it is
    /// named.
    pub id: String,
    /// The blob itself, as the phone wrote it.
    pub blob: Vec<u8>,
    /// What the secret that opens the blob takes besides what the user
    /// knows.
    pub bound: Bound,
    /// What the phone recorded about the screen lock the user set, where
    /// one is set. A user who has set none has nothing recorded.
    pub lock: Option<Lock>,
}

impl Protector {
    pub fn key_alias(&self) -> String {
        format!("synthetic_password_{}", self.id.trim_start_matches('0'))
    }
}

/// The name a phone gives the files of the protector it keeps under this
/// number: the whole of the number in hexadecimal, which it writes signed
/// and reads as the sixteen digits of a 64-bit number.
pub fn protector_name(id: i64) -> String {
    format!("{:016x}", id as u64)
}

/// What a blob is bound to besides what the user knows. A phone with a
/// weaver keeps a value in one of its slots for every user, with a screen
/// lock or without; another phone keeps random bytes beside the blob.
/// Erasing either makes the blob unopenable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    Secdiscardable(Vec<u8>),
    /// The number of the weaver's slot.
    Weaver(u32),
}

/// The weaver slot a phone recorded for a blob: the layout, then the
/// number in the order the network uses.
pub fn weaver_slot(raw: &[u8]) -> Result<u32> {
    let [layout, number @ ..] = raw else { bail!("the record of the blob's weaver slot is empty") };
    ensure!(*layout == WEAVER_SLOT_LAYOUT, "the blob's weaver slot is recorded in layout {layout}, and Skyward reads layout {WEAVER_SLOT_LAYOUT}");
    let Ok(number) = <[u8; 4]>::try_from(number) else {
        bail!("the record of the blob's weaver slot is {} bytes, and one is 5", raw.len());
    };
    let slot = i32::from_be_bytes(number);
    u32::try_from(slot).map_err(|_| anyhow::anyhow!("the blob's weaver slot is recorded as {slot}"))
}

/// What a phone recorded when the user set a screen lock: how much work it
/// takes to stretch what they type, and what the secure world recognises
/// the stretched form by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    /// Which kind of screen lock it is, in the phone's own numbering.
    pub kind: i32,
    /// How much work the stretching takes, as the powers of two the phone
    /// wrote rather than the numbers themselves.
    pub cost: [u8; 3],
    pub salt: Vec<u8>,
    /// What the secure world checks the stretched credential against. It
    /// means nothing outside the secure world.
    pub enrolled: Vec<u8>,
}

/// What the phone's own numbering calls each kind of screen lock.
impl Lock {
    pub const NONE: i32 = -1;
    pub const PATTERN: i32 = 1;
    pub const PIN: i32 = 3;
    pub const PASSWORD: i32 = 4;

    /// Read what the phone recorded. The numbers in it are written in the
    /// order the network uses, which is what the phone's own writer does.
    pub fn of(raw: &[u8]) -> Result<Lock> {
        let mut record = Record { raw, at: 0 };
        let kind = record.i32()?;
        let cost = [record.u8()?, record.u8()?, record.u8()?];
        let salt = record.counted()?.to_vec();
        let enrolled = record.counted()?.to_vec();
        Ok(Lock { kind, cost, salt, enrolled })
    }

    /// Stretch what the user typed into the secret the rest of this works
    /// from. The work is deliberately slow, and the phone chose how slow.
    pub fn stretched(&self, credential: &[u8]) -> Result<Vec<u8>> {
        let [log_n, log_r, log_p] = self.cost;
        let cost = scrypt::Params::new(log_n, 1 << log_r, 1 << log_p, STRETCHED_BYTES)
            .map_err(|e| anyhow::anyhow!("the phone asks for stretching this code cannot do: {e}"))?;
        let mut out = vec![0u8; STRETCHED_BYTES];
        scrypt::scrypt(credential, &self.salt, &cost, &mut out).map_err(|e| anyhow::anyhow!("stretch the credential: {e}"))?;
        Ok(out)
    }
}

/// Reads the numbers and runs of bytes a phone wrote one after another.
struct Record<'a> {
    raw: &'a [u8],
    at: usize,
}

impl<'a> Record<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at + n;
        if end > self.raw.len() {
            bail!("the phone's record holds {} bytes, {n} more were read at {}", self.raw.len(), self.at);
        }
        let out = &self.raw[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    /// A run of bytes, as its length and then the bytes.
    fn counted(&mut self) -> Result<&'a [u8]> {
        let len = self.i32()?;
        if len < 0 {
            bail!("the phone's record says a length of {len}");
        }
        self.take(len as usize)
    }
}

/// What the secure world is asked to check: the stretched credential under
/// its own name for that use.
pub fn gatekeeper_password(stretched: &[u8]) -> Vec<u8> {
    personalized_hash(GATEKEEPER, &[stretched])
}

/// SHA-512 of the parts behind a fixed name for what they are for.
pub fn personalized_hash(personalization: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    use sha2::{Digest, Sha512};
    let mut padded = personalization.to_vec();
    padded.resize(128, 0);
    let mut h = Sha512::new();
    h.update(&padded);
    for part in parts {
        h.update(part);
    }
    h.finalize().to_vec()
}

/// What a user who has set no credential has in place of one.
pub fn stretched_without_credential() -> Vec<u8> {
    let mut out = NO_CREDENTIAL.to_vec();
    out.resize(STRETCHED_BYTES, 0);
    out
}

/// The secret that, with the key the secure world holds, opens a blob
/// bound to the bytes kept beside it.
pub fn protector_secret(stretched: &[u8], secdiscardable: &[u8]) -> Vec<u8> {
    let mut out = stretched.to_vec();
    out.extend_from_slice(&personalized_hash(SECDISCARDABLE, &[secdiscardable]));
    out
}

/// The key a blob's weaver slot gives its value back for: the stretched
/// credential hashed under its own name, cut to the length the weaver
/// keeps keys at.
pub fn weaver_key(stretched: &[u8], key_bytes: usize) -> Result<Vec<u8>> {
    let mut key = personalized_hash(WEAVER_KEY, &[stretched]);
    ensure!(key_bytes <= key.len(), "the weaver keeps keys of {key_bytes} bytes, longer than the {} a hash gives", key.len());
    key.truncate(key_bytes);
    Ok(key)
}

/// The secret that, with the key the secure world holds, opens a blob
/// bound to a weaver slot: the stretched credential, and the hash of the
/// value the slot gave back.
pub fn weaver_protector_secret(stretched: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = stretched.to_vec();
    out.extend_from_slice(&personalized_hash(WEAVER_VALUE, &[value]));
    out
}

/// Undo the software layer of the blob, which is what the user's own
/// secret opens. The layer the secure world holds has to come off first.
pub fn open_inner(secret: &[u8], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < NONCE_BYTES + 16 {
        bail!("the user's blob is {} bytes, too few to hold anything", blob.len());
    }
    let mut key = personalized_hash(PROTECTOR_SECRET, &[secret]);
    key.truncate(KEY_BYTES);
    let (nonce, body) = blob.split_at(NONCE_BYTES);
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key))
        .decrypt(Nonce::from_slice(nonce), body)
        .map_err(|_| anyhow::anyhow!("the user's blob did not come apart; the secret is not the one it was closed with"))
}

/// What the phone put in the blob, past the two bytes that say how it is
/// written.
pub fn content(blob: &[u8]) -> Result<&[u8]> {
    let [version, kind, rest @ ..] = blob else { bail!("the user's blob says nothing about itself") };
    if *version != LATEST {
        bail!("the user's blob is written in layout {version}, and Skyward reads layout {LATEST}");
    }
    if *kind != BY_CREDENTIAL {
        bail!("the user's blob is kept behind something other than what the user knows");
    }
    Ok(rest)
}

/// The secret a user's own files are wrapped with, from the synthetic
/// password the blob held.
pub fn file_secret(synthetic_password: &[u8]) -> Vec<u8> {
    derived(synthetic_password, FILE_KEY)
}

/// What the secure world checks the user's own secret against. The keys
/// the phone keeps for a user are guarded by this rather than by what the
/// user types, so this is what a note for those keys follows from.
pub fn secret_gatekeeper_password(synthetic_password: &[u8]) -> Vec<u8> {
    derived(synthetic_password, SECRET_GATEKEEPER)
}

/// One of the keys that follow from the user's secret, under a fixed name
/// for what it is for.
fn derived(synthetic_password: &[u8], purpose: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(synthetic_password).expect("HMAC takes a key of any length");
    mac.update(&1u32.to_be_bytes());
    mac.update(purpose);
    mac.update(&[0]);
    mac.update(CONTEXT);
    mac.update(&((CONTEXT.len() * 8) as u32).to_be_bytes());
    mac.update(&256u32.to_be_bytes());
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_who_set_no_credential_has_a_stand_in_for_one() {
        let stretched = stretched_without_credential();
        assert_eq!(stretched.len(), STRETCHED_BYTES);
        assert_eq!(&stretched[..16], NO_CREDENTIAL);
        assert_eq!(&stretched[16..], [0u8; 16], "it is padded out, not repeated");
    }

    /// Checked against SHA-512 taken outside this code.
    #[test]
    fn a_name_for_what_a_hash_is_for_is_padded_to_a_whole_block() {
        let out = personalized_hash(b"secdiscardable-transform", &[b"test"]);
        assert_eq!(
            skyward_files::hex(&out),
            "b6ea00e9c56b167287e68821a4a302ee34b3d8eb43e67efd403b4ebbdfe34386\
             5c0bd2783da07e8566f8311820e57551318757cce3a3a8379ac0e1a6c72233b2"
        );
    }

    #[test]
    fn the_secret_is_the_stretched_credential_and_the_blobs_hash() {
        let secret = protector_secret(b"stretched", b"random bytes");
        assert_eq!(&secret[..9], b"stretched");
        assert_eq!(secret.len(), 9 + 64);
    }

    #[test]
    fn the_blob_says_how_it_is_written_before_anything_else() {
        assert_eq!(content(&[3, 0, 9, 9]).unwrap(), &[9, 9]);
        assert!(content(&[2, 0, 9]).is_err(), "an older layout");
        assert!(content(&[3, 1, 9]).is_err(), "kept behind something else");
        assert!(content(&[3]).is_err());
    }

    #[test]
    fn the_software_layer_opens_only_with_the_secret_it_was_closed_with() {
        let mut key = personalized_hash(PROTECTOR_SECRET, &[b"the secret"]);
        key.truncate(KEY_BYTES);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let nonce = [4u8; NONCE_BYTES];
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&cipher.encrypt(Nonce::from_slice(&nonce), &b"the password"[..]).unwrap());
        assert_eq!(open_inner(b"the secret", &blob).unwrap(), b"the password");
        assert!(open_inner(b"another secret", &blob).is_err());
        assert!(open_inner(b"the secret", &blob[..20]).is_err());
    }

    #[test]
    fn the_key_the_secure_world_holds_is_named_after_the_protector() {
        let p = Protector { id: "08a692c0f39d97c6".into(), blob: Vec::new(), bound: Bound::Weaver(0), lock: None };
        assert_eq!(p.key_alias(), "synthetic_password_8a692c0f39d97c6");
    }

    #[test]
    fn a_protectors_files_are_named_after_the_whole_number_the_phone_records() {
        assert_eq!(protector_name(0x08a6_92c0_f39d_97c6), "08a692c0f39d97c6");
        assert_eq!(protector_name(5), "0000000000000005", "the digits it is short of are zeros");
        assert_eq!(protector_name(-1), "ffffffffffffffff", "a number the phone records below zero");
    }

    #[test]
    fn a_weaver_slot_is_recorded_as_its_layout_and_its_number() {
        assert_eq!(weaver_slot(&[1, 0, 0, 0, 5]).unwrap(), 5);
        assert_eq!(weaver_slot(&[1, 0, 0, 1, 0]).unwrap(), 256, "the number in the order the network uses");
        assert!(weaver_slot(&[2, 0, 0, 0, 5]).is_err(), "another layout");
        assert!(weaver_slot(&[1, 0, 0, 5]).is_err(), "cut short");
        assert!(weaver_slot(&[1, 0xFF, 0xFF, 0xFF, 0xFF]).is_err(), "the number of no slot");
        assert!(weaver_slot(&[]).is_err());
    }

    /// Checked against SHA-512 taken outside this code, of the name padded
    /// to a whole block and then what a user with no screen lock has.
    #[test]
    fn the_weaver_key_is_the_named_hash_cut_to_the_weavers_length() {
        let stretched = stretched_without_credential();
        assert_eq!(skyward_files::hex(&weaver_key(&stretched, 16).unwrap()), "ee43b0c0aadc31e0b8ba2843de4910b3");
        assert_eq!(
            skyward_files::hex(&weaver_key(&stretched, 32).unwrap()),
            "ee43b0c0aadc31e0b8ba2843de4910b3ce7b4781a9a8c41050c4cbee46354321"
        );
        assert!(weaver_key(&stretched, 65).is_err(), "longer than a hash");
    }

    #[test]
    fn the_secret_under_a_weaver_is_the_stretched_credential_and_the_values_hash() {
        let secret = weaver_protector_secret(b"stretched", &[0xAB; 16]);
        assert_eq!(&secret[..9], b"stretched");
        assert_eq!(
            skyward_files::hex(&secret[9..]),
            "3ab55b85fd690fe840566368f54c22c3a1bd7d6ac11d75c3174b7be7dd49414e\
             a59a34c8ca70e5f2ef564d1ef8ff02bd47650f8fd34738185f9bd179ab1135fe"
        );
        assert_ne!(weaver_protector_secret(b"stretched", &[0xAB; 16]), protector_secret(b"stretched", &[0xAB; 16]));
    }

    /// The record as a phone writes it: the kind, the three powers of two
    /// the stretching costs, then the salt and the enrolment, each behind
    /// its own length.
    fn recorded(kind: i32, salt: &[u8], enrolled: &[u8]) -> Vec<u8> {
        let mut raw = kind.to_be_bytes().to_vec();
        raw.extend_from_slice(&[11, 3, 1]);
        raw.extend_from_slice(&(salt.len() as i32).to_be_bytes());
        raw.extend_from_slice(salt);
        raw.extend_from_slice(&(enrolled.len() as i32).to_be_bytes());
        raw.extend_from_slice(enrolled);
        // A phone writes how long the code is after all of it, which is
        // nothing this needs.
        raw.extend_from_slice(&6i32.to_be_bytes());
        raw
    }

    #[test]
    fn what_the_phone_recorded_about_the_screen_lock_is_read_back() {
        let raw = recorded(Lock::PIN, &[0xCD; 16], &[0xAB; 58]);
        assert_eq!(raw.len(), 93, "the length a phone writes for a code with a salt and an enrolment");
        let lock = Lock::of(&raw).unwrap();
        assert_eq!(lock.kind, Lock::PIN);
        assert_eq!(lock.cost, [11, 3, 1]);
        assert_eq!(lock.salt, vec![0xCD; 16]);
        assert_eq!(lock.enrolled, vec![0xAB; 58]);
    }

    #[test]
    fn a_record_that_ends_early_is_an_error() {
        let raw = recorded(Lock::PASSWORD, &[1; 16], &[2; 58]);
        assert!(Lock::of(&raw[..20]).is_err());
        assert!(Lock::of(&[]).is_err());
    }

    /// Checked against the second test case of RFC 7914, which gives the
    /// stretching for a password and salt with these costs. The published
    /// answer is 64 bytes and the first 32 of it are the 32 asked for here,
    /// because the last step of the stretching builds its answer in blocks.
    #[test]
    fn stretching_matches_the_published_answer_for_the_cost_the_phone_names() {
        let lock = Lock { kind: Lock::PASSWORD, cost: [10, 3, 4], salt: b"NaCl".to_vec(), enrolled: Vec::new() };
        let out = lock.stretched(b"password").unwrap();
        assert_eq!(
            skyward_files::hex(&out),
            "fdbabe1c9d3472007856e7190d01e9fe7c6ad7cbc8237830e77376634b373162"
        );
    }

    #[test]
    fn a_cost_no_machine_could_pay_is_an_error_rather_than_a_wait() {
        let lock = Lock { kind: Lock::PIN, cost: [64, 3, 1], salt: b"salt".to_vec(), enrolled: Vec::new() };
        assert!(lock.stretched(b"1234").is_err());
    }
}

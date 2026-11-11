//! Keeping what opened the data partition's users' own files, sealed with a
//! key this phone alone can derive: a file copied to another phone does not
//! open there, as the key is derived from this phone's serial number.
//!
//! What this does not protect against is someone holding this phone: they can
//! run the same code. The key is not in the TEE, because a recovery cannot ask
//! the secure world to wrap a secret it has never seen.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};

/// What a sealed credential starts with, so a file that is not one is refused
/// rather than decrypted into nonsense.
pub const MAGIC: &[u8; 8] = b"SWCRED01";
/// The bytes a salt takes in the file.
const SALT: usize = 16;
/// The bytes a nonce takes in the file.
const NONCE: usize = 12;
/// How much the file holds at least: the magic, a salt, a nonce and the tag.
pub const LEAST: usize = 8 + SALT + NONCE + 16;

/// The key the file's secret is sealed with: this phone's serial and the salt
/// it was written with, hashed once.
fn key_of(serial: &str, salt: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"skyward credential v1|");
    hasher.update(serial.as_bytes());
    hasher.update(b"|");
    hasher.update(salt);
    hasher.finalize().into()
}

/// `secret`, sealed for this phone. The salt is fresh each time, so two
/// phones that keep the same credential keep two different files.
pub fn seal(serial: &str, secret: &[u8]) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT];
    let mut nonce = [0u8; NONCE];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    let key = key_of(serial, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).context("make the cipher")?;
    let body = cipher
        .encrypt(Nonce::from_slice(&nonce), secret)
        .map_err(|_| anyhow!("the credential could not be sealed"))?;
    let mut out = Vec::with_capacity(LEAST + secret.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(out)
}

/// The secret sealed in `file`, this phone's to open. The tag is checked, so
/// a file another phone wrote, or one changed by a byte, says why instead of
/// opening as something else.
pub fn open(serial: &str, file: &[u8]) -> Result<Vec<u8>> {
    if file.len() < LEAST {
        return Err(anyhow!("it is {} bytes, too short to be a credential", file.len()));
    }
    if &file[..8] != MAGIC {
        return Err(anyhow!("it does not start as a credential Skyward wrote"));
    }
    let salt = &file[8..8 + SALT];
    let nonce = &file[8 + SALT..8 + SALT + NONCE];
    let body = &file[8 + SALT + NONCE..];
    let key = key_of(serial, salt);
    let cipher = Aes256Gcm::new_from_slice(&key).context("make the cipher")?;
    cipher
        .decrypt(Nonce::from_slice(nonce), body)
        .map_err(|_| anyhow!("it was written by another phone, or changed since"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_opens_on_the_phone_that_sealed_it() {
        let sealed = seal("507d820c", b"1234").unwrap();
        assert_eq!(open("507d820c", &sealed).unwrap(), b"1234".to_vec());
    }

    #[test]
    fn another_phones_serial_does_not_open_it() {
        let sealed = seal("507d820c", b"1234").unwrap();
        assert!(open("otherserial", &sealed).is_err());
    }

    #[test]
    fn a_changed_byte_does_not_open_it() {
        let mut sealed = seal("507d820c", b"1234").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(open("507d820c", &sealed).is_err());
    }
}

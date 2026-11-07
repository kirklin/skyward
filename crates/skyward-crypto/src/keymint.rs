use crate::token::Token;
use anyhow::{Context, Result};
use skyward_binder::driver::{Binder, Handle};
use skyward_binder::parcel::Parcel;
use skyward_binder::{service, status};

/// The phone's own key service, the only thing that can use a key the
/// hardware wrapped.
pub const SERVICE: &str = "android.hardware.security.keymint.IKeyMintDevice/default";

const DEVICE: &str = "android.hardware.security.keymint.IKeyMintDevice";
const OPERATION: &str = "android.hardware.security.keymint.IKeyMintOperation";

const GET_HARDWARE_INFO: u32 = 1;
const UPGRADE_KEY: u32 = 6;
const CONVERT_STORAGE_KEY: u32 = 13;
const GET_KEY_CHARACTERISTICS: u32 = 14;
const BEGIN: u32 = 10;

/// The top of a tag says what kind of value it carries.
const TAG_KIND: i32 = -268_435_456;
const KIND_ULONG: i32 = 1_342_177_280;
const KIND_ULONG_REP: i32 = -1_610_612_736;
const KIND_DATE: i32 = 1_610_612_736;
const KIND_BYTES: i32 = -1_879_048_192;
const KIND_BIGNUM: i32 = -2_147_483_648;
const UPDATE: u32 = 2;
const FINISH: u32 = 3;
const ABORT: u32 = 4;

/// How long the nonce of an AES-GCM key is here.
const NONCE_BYTES: usize = 12;

/// The key service's way of saying the key was made under an older build of
/// the phone's own firmware and has to be renewed before it will be used.
const KEY_REQUIRES_UPGRADE: i32 = -62;

const PURPOSE_DECRYPT: i32 = 1;
const BLOCK_MODE_GCM: i32 = 32;
const PADDING_NONE: i32 = 1;

/// What a key parameter is tagged with. The high bits say which kind of
/// value follows, which is why the numbers look arbitrary.
const TAG_PURPOSE: i32 = 536_870_913;
const TAG_BLOCK_MODE: i32 = 536_870_916;
const TAG_PADDING: i32 = 536_870_918;
const TAG_MAC_LENGTH: i32 = 805_307_371;
const TAG_APPLICATION_ID: i32 = -1_879_047_591;
const TAG_NONCE: i32 = -1_879_047_191;

/// Which arm of a parameter's value is used.
const VALUE_PADDING_MODE: i32 = 3;
const VALUE_KEY_PURPOSE: i32 = 7;
const VALUE_INTEGER: i32 = 11;
const VALUE_BLOB: i32 = 14;
const VALUE_BLOCK_MODE: i32 = 2;

/// The tag's value and which arm it is written as.
enum Value<'a> {
    Enum(i32, i32),
    Integer(i32),
    Blob(&'a [u8]),
}

fn parameter(p: &mut Parcel, tag: i32, value: Value) {
    // Each parameter is a structure of its own: it says it is there, then
    // how long it is, so the far side can step over what it does not know.
    p.i32(1);
    let start = p.here();
    p.i32(0);
    p.i32(tag);
    // The value is a structure of its own too, and says so the same way,
    // though it carries no length because only one arm of it is ever set.
    p.i32(1);
    match value {
        Value::Enum(arm, v) => {
            p.i32(arm);
            p.i32(v);
        }
        Value::Integer(v) => {
            p.i32(VALUE_INTEGER);
            p.i32(v);
        }
        Value::Blob(v) => {
            p.i32(VALUE_BLOB);
            p.blob(v);
        }
    }
    let size = (p.here() - start) as i32;
    p.fill(start, size);
}

/// Hand over the note the secure world wrote when the user answered their
/// screen lock. A key guarded by the screen lock is refused without it.
fn vouched(p: &mut Parcel, note: &Token) {
    p.i32(1);
    let start = p.here();
    p.i32(0);
    p.i64(note.challenge);
    p.i64(note.user);
    p.i64(note.authenticator);
    p.i32(note.kind);
    // The time it was written is a structure of its own.
    p.i32(1);
    let when = p.here();
    p.i32(0);
    p.i64(note.when);
    let size = (p.here() - when) as i32;
    p.fill(when, size);
    p.blob(&note.mac);
    let size = (p.here() - start) as i32;
    p.fill(start, size);
}

/// What the phone says about its key hardware.
pub struct Hardware {
    pub version: i32,
    pub security_level: i32,
    pub name: String,
    pub author: String,
}

/// The phone's key service, as this program talks to it.
pub struct KeyMint<'a> {
    device: Handle<'a>,
}

impl<'a> KeyMint<'a> {
    /// Reach the running key service, or say that none is.
    pub fn open(binder: &'a Binder) -> Result<KeyMint<'a>> {
        let device = service::find(binder, SERVICE)?
            .with_context(|| format!("{SERVICE} is not running; the phone's key service has to be started first"))?;
        Ok(KeyMint { device })
    }

    pub fn hardware(&self) -> Result<Hardware> {
        let reply = self.device.transact(GET_HARDWARE_INFO, &Parcel::call(DEVICE)).context("ask the key service about itself")?;
        let mut read = reply.read();
        status::ok(&mut read).context("ask the key service about itself")?;
        let end = read.structure()?;
        let info = Hardware {
            version: read.i32()?,
            security_level: read.i32()?,
            name: read.string()?,
            author: read.string()?,
        };
        read.go(end)?;
        Ok(info)
    }

    /// Turn a key that only the storage hardware can use into the form it
    /// takes for this start of the phone. The key itself never leaves the
    /// secure world: what comes back is wrapped again, for the hardware to
    /// unwrap as it reads and writes.
    pub fn ephemeral_storage_key(&self, key: &[u8]) -> Result<Vec<u8>> {
        match self.convert(key) {
            Err(e) if status::refused_with(&e) == Some(KEY_REQUIRES_UPGRADE) => {
                // As with a key that decrypts, the key is renewed for the
                // firmware running now, as Android's keystore2 renews one
                // for vold, and the one on disk is left as it is. A storage
                // key is bound to nothing that has to be given again.
                let renewed = self.upgrade(key, |call| call.i32(0)).context("renew the storage key for the firmware running now")?;
                self.convert(&renewed)
            }
            converted => converted,
        }
    }

    fn convert(&self, key: &[u8]) -> Result<Vec<u8>> {
        let mut call = Parcel::call(DEVICE);
        call.blob(key);
        let reply = self.device.transact(CONVERT_STORAGE_KEY, &call).context("make a storage key for this start")?;
        let mut read = reply.read();
        status::ok(&mut read).context("make a storage key for this start")?;
        read.blob().context("read the storage key")
    }

    /// What the key service says a key may be used for: each entry is a tag
    /// and the number or length that came with it.
    pub fn authorizations(&self, key_blob: &[u8], app_id: &[u8]) -> Result<Vec<(i32, i64)>> {
        let mut call = Parcel::call(DEVICE);
        call.blob(key_blob);
        call.blob(app_id);
        call.blob(&[]);
        let reply = self.device.transact(GET_KEY_CHARACTERISTICS, &call).context("ask what the key may be used for")?;
        let mut read = reply.read();
        status::ok(&mut read).context("ask what the key may be used for")?;
        let mut out = Vec::new();
        for _ in 0..read.i32()?.max(0) {
            let end = read.structure()?;
            read.i32().context("read which part of the phone holds the key")?;
            for _ in 0..read.i32()?.max(0) {
                let next = read.structure()?;
                let tag = read.i32()?;
                read.i32().context("read whether the tag has a value")?;
                read.i32().context("read which arm of the value is set")?;
                let value = match tag & TAG_KIND {
                    KIND_ULONG | KIND_ULONG_REP | KIND_DATE => read.i64()?,
                    KIND_BYTES | KIND_BIGNUM => read.blob()?.len() as i64,
                    _ => read.i32()? as i64,
                };
                out.push((tag, value));
                read.go(next)?;
            }
            read.go(end)?;
        }
        Ok(out)
    }

    /// Undo an AES-GCM encryption the key service did with `key_blob`. The
    /// service only hands back the plaintext when `app_id` is the one the
    /// key was made for, so a wrong secret fails here rather than yielding
    /// something that looks like a key. A key guarded by a screen lock
    /// takes the note the secure world wrote when it was answered.
    pub fn gcm_decrypt(&self, key_blob: &[u8], app_id: &[u8], nonce: &[u8], body: &[u8], note: Option<&Token>) -> Result<Vec<u8>> {
        let operation = match self.begin(key_blob, app_id, nonce, note) {
            Ok(operation) => operation,
            Err(e) if status::refused_with(&e) == Some(KEY_REQUIRES_UPGRADE) => {
                // The phone's firmware has moved on since this key was
                // made. Renewing it gives a key for the firmware running
                // now; the one on disk is left as it is, so nothing of the
                // phone's own is changed by reading it here.
                let renewed = self
                    .upgrade(key_blob, |call| Self::gcm_params(call, app_id, nonce))
                    .context("renew the key for the firmware running now")?;
                self.begin(&renewed, app_id, nonce, note)?
            }
            Err(e) => return Err(e),
        };
        let mut out = self.update(&operation, body)?;
        out.extend_from_slice(&self.finish(&operation)?);
        Ok(out)
    }

    /// What the key service has to be told to renew an AES-GCM key: how
    /// long its tag is, what it is bound to, and the nonce it was encrypted
    /// under. A decryption is told the same and the purpose it is opened
    /// for. Renewal is not told one: its call has no purpose to name, and
    /// the key service refuses one among its parameters as an invalid
    /// argument, as Android's keystore2 leaves it out of what it renews
    /// vold's key with.
    fn gcm_params(call: &mut Parcel, app_id: &[u8], nonce: &[u8]) {
        call.i32(if app_id.is_empty() { 4 } else { 5 });
        Self::gcm_key_params(call, app_id, nonce);
    }

    /// The same parameters with the purpose a decryption is opened for.
    fn decrypt_params(call: &mut Parcel, app_id: &[u8], nonce: &[u8]) {
        call.i32(if app_id.is_empty() { 5 } else { 6 });
        Self::gcm_key_params(call, app_id, nonce);
        parameter(call, TAG_PURPOSE, Value::Enum(VALUE_KEY_PURPOSE, PURPOSE_DECRYPT));
    }

    /// How long the key's tag is, what it is bound to, and the nonce it was
    /// encrypted under.
    ///
    /// What a key is bound to is named only where there is something: the
    /// secure world folds the binding into the key it unwraps the key blob
    /// with, and a binding to nothing named is not the same to it as none
    /// named at all, so naming an empty one leaves it unable to unwrap a
    /// key that was made bound to nothing.
    fn gcm_key_params(call: &mut Parcel, app_id: &[u8], nonce: &[u8]) {
        parameter(call, TAG_BLOCK_MODE, Value::Enum(VALUE_BLOCK_MODE, BLOCK_MODE_GCM));
        parameter(call, TAG_PADDING, Value::Enum(VALUE_PADDING_MODE, PADDING_NONE));
        parameter(call, TAG_MAC_LENGTH, Value::Integer(128));
        if !app_id.is_empty() {
            parameter(call, TAG_APPLICATION_ID, Value::Blob(app_id));
        }
        parameter(call, TAG_NONCE, Value::Blob(nonce));
    }

    /// Open a decryption with the key service and keep hold of it.
    fn begin(&self, key_blob: &[u8], app_id: &[u8], nonce: &[u8], note: Option<&Token>) -> Result<Handle<'a>> {
        let mut call = Parcel::call(DEVICE);
        call.i32(PURPOSE_DECRYPT);
        call.blob(key_blob);
        Self::decrypt_params(&mut call, app_id, nonce);
        // A key guarded by the screen lock takes the note the secure world
        // wrote when the user answered it; one that is not guarded takes
        // nothing, and is refused nothing for carrying a note either.
        match note {
            Some(note) => vouched(&mut call, note),
            None => call.none(),
        }

        // The decryption the service opens outlives the reply that named
        // it, so take a share of it before the reply's room goes back.
        let reply = self.device.transact(BEGIN, &call).context("start a decryption")?;
        let mut read = reply.read();
        status::ok(&mut read).context("start a decryption")?;
        read.structure().context("read what the key service answered")?;
        read.i64().context("read the decryption's challenge")?;
        let params = read.i32().context("read how many parameters the decryption came back with")?;
        for _ in 0..params.max(0) {
            let end = read.structure().context("read a parameter the decryption came back with")?;
            read.go(end)?;
        }
        match read.object().context("read the decryption the key service opened")? {
            Some(h) => Handle::hold(reply.binder(), h),
            None => anyhow::bail!("the key service opened no decryption"),
        }
    }

    /// A key for the firmware running now, from one made under an older
    /// build. What it is bound to has to be given again, as `params` write
    /// it, or the service cannot tell it is the right key.
    fn upgrade(&self, key_blob: &[u8], params: impl FnOnce(&mut Parcel)) -> Result<Vec<u8>> {
        let mut call = Parcel::call(DEVICE);
        call.blob(key_blob);
        params(&mut call);
        let reply = self.device.transact(UPGRADE_KEY, &call).context("renew the key")?;
        let mut read = reply.read();
        status::ok(&mut read).context("renew the key")?;
        read.blob().context("read the renewed key")
    }

    fn update(&self, operation: &Handle, input: &[u8]) -> Result<Vec<u8>> {
        let mut call = Parcel::call(OPERATION);
        call.blob(input);
        call.none();
        call.none();
        let reply = operation.transact(UPDATE, &call).context("feed the decryption")?;
        let mut read = reply.read();
        status::ok(&mut read).context("feed the decryption")?;
        read.blob().context("read what the decryption gave back")
    }

    fn finish(&self, operation: &Handle) -> Result<Vec<u8>> {
        let mut call = Parcel::call(OPERATION);
        call.opt_blob(None);
        call.opt_blob(None);
        call.none();
        call.none();
        call.opt_blob(None);
        let reply = operation.transact(FINISH, &call).context("close the decryption")?;
        let mut read = reply.read();
        status::ok(&mut read).context("close the decryption, which is where a wrong key is refused")?;
        read.blob().context("read what the decryption ended with")
    }

    fn abort(&self, operation: &Handle) -> Result<()> {
        let reply = operation.transact(ABORT, &Parcel::call(OPERATION)).context("drop the decryption")?;
        status::ok(&mut reply.read()).context("drop the decryption")
    }

    /// Whether the secure world will unwrap the key at all: a decryption is
    /// opened on it under a nonce of nothing and dropped again, which reads
    /// nothing and changes nothing. It tells a key the secure world cannot
    /// unwrap, as one made under another build or bound to something not
    /// given back to it is, apart from one that merely decrypts to nothing.
    pub fn usable(&self, key_blob: &[u8], app_id: &[u8]) -> Result<()> {
        let operation = self.begin(key_blob, app_id, &[0; NONCE_BYTES], None)?;
        self.abort(&operation)
    }
}

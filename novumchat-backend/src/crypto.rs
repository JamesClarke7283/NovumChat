//! Optional encryption at rest for the chat database.
//!
//! * A random 256-bit **data key** encrypts every sensitive column (titles,
//!   system prompts, message content and metadata, attachment names and bytes)
//!   with **AES-256-GCM** and a fresh 96-bit nonce per value.
//! * The data key is wrapped with a **key-encryption key derived from your
//!   passphrase with Argon2id** (64 MiB, 3 passes by default) and a random salt.
//! * Only the wrapped key, the salt and the KDF parameters are stored. Changing
//!   the passphrase re-wraps the data key; nothing else is re-encrypted.
//!
//! Encrypted text values look like `enc1:<base64(nonce ‖ ciphertext ‖ tag)>`,
//! encrypted blobs start with the bytes `NCE1`. Values without the marker are
//! plaintext, which lets encryption be switched on and off in place.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Result, anyhow, bail};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use novumchat_core::types::VaultStatus;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const TEXT_PREFIX: &str = "enc1:";
pub const BLOB_MAGIC: &[u8; 4] = b"NCE1";
const NONCE_LEN: usize = 12;
const DEK_AAD: &[u8] = b"novumchat:data-key:v1";

pub type SecretKey = Zeroizing<[u8; 32]>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory cost in KiB.
    pub m: u32,
    /// Iterations.
    pub t: u32,
    /// Parallelism.
    pub p: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self { m: 64 * 1024, t: 3, p: 1 }
    }
}

/// Stored in the `kv` table under [`VaultHeader::KEY`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultHeader {
    pub version: u32,
    pub kdf: String,
    pub params: KdfParams,
    pub salt: String,
    /// AES-256-GCM(KEK, data key), base64 of nonce ‖ ciphertext ‖ tag.
    pub wrapped_key: String,
}

impl VaultHeader {
    pub const KEY: &'static str = "vault";
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("operating system RNG unavailable");
    buf
}

fn derive_kek(passphrase: &str, salt: &[u8], params: &KdfParams) -> Result<SecretKey> {
    let p = Params::new(params.m, params.t, params.p, Some(32)).map_err(|e| anyhow!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = Zeroizing::new([0u8; 32]);
    argon.hash_password_into(passphrase.as_bytes(), salt, out.as_mut()).map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(out)
}

fn seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes = random_bytes::<NONCE_LEN>();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), Payload { msg: plaintext, aad })
        .expect("AES-GCM encryption cannot fail for in-memory buffers");
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    out
}

fn open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < NONCE_LEN + 16 {
        bail!("ciphertext too short");
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| anyhow!("decryption failed (wrong key or corrupted data)"))
}

/// Creates a new header and data key for `passphrase`.
pub fn create_vault(passphrase: &str, params: KdfParams) -> Result<(VaultHeader, SecretKey)> {
    validate_passphrase(passphrase)?;
    let salt = random_bytes::<16>();
    let kek = derive_kek(passphrase, &salt, &params)?;
    let dek: SecretKey = Zeroizing::new(random_bytes::<32>());
    let wrapped = seal(&kek, DEK_AAD, dek.as_ref());
    let header = VaultHeader {
        version: 1,
        kdf: "argon2id".into(),
        params,
        salt: B64.encode(salt),
        wrapped_key: B64.encode(wrapped),
    };
    Ok((header, dek))
}

/// Recovers the data key from `header` (fails on a wrong passphrase).
pub fn unwrap_key(header: &VaultHeader, passphrase: &str) -> Result<SecretKey> {
    if header.kdf != "argon2id" || header.version != 1 {
        bail!("unsupported vault format");
    }
    let salt = B64.decode(&header.salt)?;
    let wrapped = B64.decode(&header.wrapped_key)?;
    let kek = derive_kek(passphrase, &salt, &header.params)?;
    let dek = open(&kek, DEK_AAD, &wrapped).map_err(|_| anyhow!("Incorrect passphrase"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    if dek.len() != 32 {
        bail!("corrupted vault key");
    }
    key.copy_from_slice(&dek);
    Ok(key)
}

/// Re-wraps the data key under a new passphrase (fresh salt).
pub fn rewrap(header: &VaultHeader, old: &str, new: &str) -> Result<VaultHeader> {
    validate_passphrase(new)?;
    let dek = unwrap_key(header, old)?;
    let salt = random_bytes::<16>();
    let kek = derive_kek(new, &salt, &header.params)?;
    Ok(VaultHeader {
        salt: B64.encode(salt),
        wrapped_key: B64.encode(seal(&kek, DEK_AAD, dek.as_ref())),
        ..header.clone()
    })
}

pub fn validate_passphrase(p: &str) -> Result<()> {
    if p.chars().count() < 8 {
        bail!("Use a passphrase of at least 8 characters");
    }
    Ok(())
}

/// Encrypts/decrypts individual values with the data key.
#[derive(Clone)]
pub struct FieldCipher {
    key: SecretKey,
}

impl FieldCipher {
    pub fn new(key: SecretKey) -> Self {
        Self { key }
    }

    pub fn seal_str(&self, ctx: &str, s: &str) -> String {
        format!("{TEXT_PREFIX}{}", B64.encode(seal(&self.key, ctx.as_bytes(), s.as_bytes())))
    }

    pub fn open_str(&self, ctx: &str, s: &str) -> Result<String> {
        let body = s.strip_prefix(TEXT_PREFIX).ok_or_else(|| anyhow!("not an encrypted value"))?;
        let raw = B64.decode(body)?;
        Ok(String::from_utf8(open(&self.key, ctx.as_bytes(), &raw)?)?)
    }

    pub fn seal_bytes(&self, ctx: &str, b: &[u8]) -> Vec<u8> {
        let mut out = BLOB_MAGIC.to_vec();
        out.extend(seal(&self.key, ctx.as_bytes(), b));
        out
    }

    pub fn open_bytes(&self, ctx: &str, b: &[u8]) -> Result<Vec<u8>> {
        let body = b.strip_prefix(BLOB_MAGIC.as_slice()).ok_or_else(|| anyhow!("not an encrypted blob"))?;
        open(&self.key, ctx.as_bytes(), body)
    }
}

/// Value codec used by the store: encrypts when a cipher is present and
/// transparently reads both plaintext and encrypted values.
#[derive(Clone, Default)]
pub struct Codec(pub Option<FieldCipher>);

impl Codec {
    pub fn plain() -> Self {
        Self(None)
    }

    pub fn enc(&self, ctx: &str, s: &str) -> String {
        match &self.0 {
            Some(c) => c.seal_str(ctx, s),
            None => s.to_string(),
        }
    }

    pub fn enc_opt(&self, ctx: &str, s: Option<&str>) -> Option<String> {
        s.map(|s| self.enc(ctx, s))
    }

    pub fn dec(&self, ctx: &str, s: &str) -> Result<String> {
        if s.starts_with(TEXT_PREFIX) {
            match &self.0 {
                Some(c) => c.open_str(ctx, s),
                None => Err(anyhow!("The chat database is locked")),
            }
        } else {
            Ok(s.to_string())
        }
    }

    pub fn dec_opt(&self, ctx: &str, s: Option<&str>) -> Result<Option<String>> {
        s.map(|s| self.dec(ctx, s)).transpose()
    }

    pub fn enc_bytes(&self, ctx: &str, b: &[u8]) -> Vec<u8> {
        match &self.0 {
            Some(c) => c.seal_bytes(ctx, b),
            None => b.to_vec(),
        }
    }

    pub fn dec_bytes(&self, ctx: &str, b: &[u8]) -> Result<Vec<u8>> {
        if b.starts_with(BLOB_MAGIC) {
            match &self.0 {
                Some(c) => c.open_bytes(ctx, b),
                None => Err(anyhow!("The chat database is locked")),
            }
        } else {
            Ok(b.to_vec())
        }
    }
}

/// In-memory vault state.
#[derive(Default)]
pub struct Vault {
    inner: RwLock<VaultInner>,
}

#[derive(Default)]
struct VaultInner {
    header: Option<VaultHeader>,
    key: Option<SecretKey>,
}

impl Vault {
    pub fn new(header: Option<VaultHeader>) -> Self {
        Self { inner: RwLock::new(VaultInner { header, key: None }) }
    }

    pub fn status(&self) -> VaultStatus {
        let g = self.inner.read();
        VaultStatus { encrypted: g.header.is_some(), unlocked: g.header.is_none() || g.key.is_some() }
    }

    pub fn header(&self) -> Option<VaultHeader> {
        self.inner.read().header.clone()
    }

    /// The codec for reads/writes, or an error while locked.
    pub fn codec(&self) -> Result<Codec> {
        let g = self.inner.read();
        match (&g.header, &g.key) {
            (None, _) => Ok(Codec::plain()),
            (Some(_), Some(k)) => Ok(Codec(Some(FieldCipher::new(k.clone())))),
            (Some(_), None) => Err(anyhow!("The chat database is locked")),
        }
    }

    pub fn set_unlocked(&self, key: SecretKey) {
        self.inner.write().key = Some(key);
    }

    pub fn lock(&self) {
        self.inner.write().key = None;
    }

    pub fn install(&self, header: Option<VaultHeader>, key: Option<SecretKey>) {
        let mut g = self.inner.write();
        g.header = header;
        g.key = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams { m: 1024, t: 1, p: 1 }
    }

    #[test]
    fn wrap_unwrap_and_wrong_passphrase() {
        let (header, key) = create_vault("correct horse battery", fast()).unwrap();
        let again = unwrap_key(&header, "correct horse battery").unwrap();
        assert_eq!(key.as_ref(), again.as_ref());
        assert!(unwrap_key(&header, "wrong passphrase!").is_err());
        let rewrapped = rewrap(&header, "correct horse battery", "new passphrase 123").unwrap();
        assert_ne!(rewrapped.salt, header.salt);
        let k2 = unwrap_key(&rewrapped, "new passphrase 123").unwrap();
        assert_eq!(key.as_ref(), k2.as_ref());
        assert!(unwrap_key(&rewrapped, "correct horse battery").is_err());
        assert!(create_vault("short", fast()).is_err());
    }

    #[test]
    fn field_round_trip_and_context_binding() {
        let (_, key) = create_vault("correct horse battery", fast()).unwrap();
        let codec = Codec(Some(FieldCipher::new(key)));
        let sealed = codec.enc("message.content", "secret ✓ 日本");
        assert!(sealed.starts_with(TEXT_PREFIX));
        assert!(!sealed.contains("secret"));
        assert_eq!(codec.dec("message.content", &sealed).unwrap(), "secret ✓ 日本");
        // A value cannot be moved to another column.
        assert!(codec.dec("chat.title", &sealed).is_err());
        // Same plaintext encrypts differently each time.
        assert_ne!(sealed, codec.enc("message.content", "secret ✓ 日本"));
        // Plaintext passes through.
        assert_eq!(codec.dec("x", "hello").unwrap(), "hello");
        // Blobs.
        let blob = codec.enc_bytes("attachment.data", &[1, 2, 3]);
        assert!(blob.starts_with(BLOB_MAGIC));
        assert_eq!(codec.dec_bytes("attachment.data", &blob).unwrap(), vec![1, 2, 3]);
        // Locked codec refuses encrypted values.
        assert!(Codec::plain().dec("message.content", &sealed).is_err());
    }

    #[test]
    fn vault_states() {
        let v = Vault::new(None);
        assert_eq!(v.status(), VaultStatus { encrypted: false, unlocked: true });
        let (header, key) = create_vault("correct horse battery", fast()).unwrap();
        v.install(Some(header), None);
        assert_eq!(v.status(), VaultStatus { encrypted: true, unlocked: false });
        assert!(v.codec().is_err());
        v.set_unlocked(key);
        assert!(v.status().unlocked);
        assert!(v.codec().unwrap().0.is_some());
        v.lock();
        assert!(v.codec().is_err());
    }
}

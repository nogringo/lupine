//! Envelope: `version (1) || nonce (24) || XChaCha20-Poly1305 ciphertext || tag (16)`.

use std::fmt;

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{Tag, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Error, Result};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const ENVELOPE_VERSION: u8 = 1;
const HEADER_LEN: usize = 1 + NONCE_LEN;
pub const ENVELOPE_OVERHEAD: usize = HEADER_LEN + TAG_LEN;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("the OS random number generator failed");
    out
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SymmetricKey([u8; KEY_LEN]);

impl SymmetricKey {
    pub fn generate() -> Self {
        Self(random_bytes())
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let bytes: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| Error::Malformed("key must be 32 bytes"))?;
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// HKDF-SHA256 without salt. `info` is the domain separation label.
    pub fn derive(&self, info: &[u8]) -> Self {
        let mut out = [0u8; KEY_LEN];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(info, &mut out)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self(out)
    }

    pub fn mac(&self, data: &[u8]) -> [u8; 32] {
        self.mac_parts([data])
    }

    pub fn mac_parts<'a>(&self, parts: impl IntoIterator<Item = &'a [u8]>) -> [u8; 32] {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC accepts keys of any length");
        for part in parts {
            mac.update(part);
        }
        mac.finalize().into_bytes().into()
    }

    pub(crate) fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&self.0.into())
    }
}

impl fmt::Debug for SymmetricKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SymmetricKey(<redacted>)")
    }
}

/// Builds an envelope in a single allocation: the caller writes the plaintext
/// straight into `body_mut()` (from a file read, for instance) and `seal`
/// encrypts it in place.
pub struct SealBuffer {
    buf: Vec<u8>,
}

impl SealBuffer {
    /// The body starts zeroed, which doubles as padding.
    pub fn new(body_len: usize) -> Self {
        Self {
            buf: vec![0u8; ENVELOPE_OVERHEAD + body_len],
        }
    }

    pub fn body(&self) -> &[u8] {
        let end = self.buf.len() - TAG_LEN;
        &self.buf[HEADER_LEN..end]
    }

    pub fn body_mut(&mut self) -> &mut [u8] {
        let end = self.buf.len() - TAG_LEN;
        &mut self.buf[HEADER_LEN..end]
    }

    pub fn seal(mut self, key: &SymmetricKey, nonce: &[u8; NONCE_LEN], aad: &[u8]) -> Vec<u8> {
        self.buf[0] = ENVELOPE_VERSION;
        self.buf[1..HEADER_LEN].copy_from_slice(nonce);
        let end = self.buf.len() - TAG_LEN;
        let tag = key
            .cipher()
            .encrypt_inout_detached(
                &XNonce::from(*nonce),
                aad,
                (&mut self.buf[HEADER_LEN..end]).into(),
            )
            .expect("XChaCha20-Poly1305 accepts messages up to 256 GiB");
        self.buf[end..].copy_from_slice(&tag);
        self.buf
    }
}

pub fn seal(key: &SymmetricKey, nonce: &[u8; NONCE_LEN], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut buffer = SealBuffer::new(plaintext.len());
    buffer.body_mut().copy_from_slice(plaintext);
    buffer.seal(key, nonce, aad)
}

/// Decrypts in place and returns the body, padding included.
pub fn open(key: &SymmetricKey, aad: &[u8], mut envelope: Vec<u8>) -> Result<Vec<u8>> {
    if envelope.len() < ENVELOPE_OVERHEAD {
        return Err(Error::Malformed("envelope too short"));
    }
    if envelope[0] != ENVELOPE_VERSION {
        return Err(Error::UnsupportedVersion(envelope[0]));
    }
    let end = envelope.len() - TAG_LEN;
    let nonce: [u8; NONCE_LEN] = envelope[1..HEADER_LEN].try_into().expect("length checked");
    let tag: [u8; TAG_LEN] = envelope[end..].try_into().expect("length checked");
    key.cipher()
        .decrypt_inout_detached(
            &XNonce::from(nonce),
            aad,
            (&mut envelope[HEADER_LEN..end]).into(),
            &Tag::from(tag),
        )
        .map_err(|_| Error::Decrypt)?;
    envelope.truncate(end);
    envelope.drain(..HEADER_LEN);
    Ok(envelope)
}

/// Padmé (Nikitin et al., PETS 2019): rounds `len` up so that at most
/// O(log log len) bits of it leak, for at most 12% overhead.
pub fn padme(len: u64) -> u64 {
    if len < 2 {
        return len;
    }
    let exponent = 63 - u64::from(len.leading_zeros());
    let exponent_bits = 64 - u64::from(exponent.leading_zeros());
    let mask = (1u64 << (exponent - exponent_bits)) - 1;
    (len + mask) & !mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonce() -> [u8; NONCE_LEN] {
        random_bytes()
    }

    #[test]
    fn seal_open_round_trip() {
        let key = SymmetricKey::generate();
        for len in [0usize, 1, 31, 4096, 100_000] {
            let plaintext: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let envelope = seal(&key, &nonce(), b"aad", &plaintext);
            assert_eq!(envelope.len(), len + ENVELOPE_OVERHEAD);
            assert_eq!(open(&key, b"aad", envelope).unwrap(), plaintext);
        }
    }

    #[test]
    fn every_tampered_byte_is_rejected() {
        let key = SymmetricKey::generate();
        let envelope = seal(&key, &nonce(), b"aad", b"hello");
        for i in 1..envelope.len() {
            let mut tampered = envelope.clone();
            tampered[i] ^= 0x01;
            assert!(matches!(open(&key, b"aad", tampered), Err(Error::Decrypt)));
        }
        let mut wrong_version = envelope.clone();
        wrong_version[0] = 2;
        assert!(matches!(
            open(&key, b"aad", wrong_version),
            Err(Error::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn wrong_key_or_aad_is_rejected() {
        let key = SymmetricKey::generate();
        let envelope = seal(&key, &nonce(), b"aad", b"hello");
        assert!(open(&SymmetricKey::generate(), b"aad", envelope.clone()).is_err());
        assert!(open(&key, b"other", envelope).is_err());
    }

    #[test]
    fn the_nonce_is_the_callers() {
        let key = SymmetricKey::generate();
        let nonce = nonce();
        let envelope = seal(&key, &nonce, b"", b"same");
        assert_eq!(envelope[1..HEADER_LEN], nonce);
        assert_eq!(envelope, seal(&key, &nonce, b"", b"same"));
    }

    #[test]
    fn derive_separates_domains() {
        let root = SymmetricKey::from_bytes([7; 32]);
        assert_eq!(root.derive(b"a").as_bytes(), root.derive(b"a").as_bytes());
        assert_ne!(root.derive(b"a").as_bytes(), root.derive(b"b").as_bytes());
    }

    #[test]
    fn padme_known_values() {
        assert_eq!(padme(0), 0);
        assert_eq!(padme(1), 1);
        assert_eq!(padme(9), 10);
        assert_eq!(padme(1000), 1024);
        assert_eq!(padme(1_000_000), 1_015_808);
        assert_eq!(padme(8 << 20), 8 << 20);
    }

    #[test]
    fn padme_bounds() {
        let mut previous = 0;
        for len in (1..2_000_000u64).step_by(997) {
            let padded = padme(len);
            assert!(padded >= len);
            assert!(padded >= previous, "padme must be monotonic");
            assert!((padded - len) as f64 <= len as f64 * 0.12 + 1.0);
            previous = padded;
        }
    }
}

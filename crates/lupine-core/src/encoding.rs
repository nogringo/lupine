//! Serde helpers for binary fields carried as base64 in JSON.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serializer};

use crate::crypto::SymmetricKey;

pub fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    STANDARD.decode(text).ok()
}

pub mod base64_opt {
    use super::*;

    pub fn serialize<S: Serializer>(value: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => s.serialize_str(&encode(bytes)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|text| decode(&text).ok_or_else(|| serde::de::Error::custom("invalid base64")))
            .transpose()
    }
}

pub mod key {
    use super::*;

    pub fn serialize<S: Serializer>(key: &SymmetricKey, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(key.as_bytes()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SymmetricKey, D::Error> {
        let text = String::deserialize(d)?;
        decode(&text)
            .and_then(|bytes| SymmetricKey::from_slice(&bytes).ok())
            .ok_or_else(|| serde::de::Error::custom("invalid key"))
    }
}

/// Secp256k1 secret keys as hex.
pub mod secret_key {
    use nostr::key::SecretKey;

    use super::*;

    pub fn serialize<S: Serializer>(key: &SecretKey, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&key.to_secret_hex())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretKey, D::Error> {
        let text = String::deserialize(d)?;
        SecretKey::from_hex(&text).map_err(|_| serde::de::Error::custom("invalid secret key"))
    }
}

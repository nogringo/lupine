use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::crypto::random_bytes;
use crate::error::Error;

macro_rules! hex_id {
    ($(#[$meta:meta])* $name:ident, $len:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub [u8; $len]);

        impl $name {
            pub const LEN: usize = $len;

            pub fn as_bytes(&self) -> &[u8; $len] {
                &self.0
            }

            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self, Error> {
                let mut bytes = [0u8; $len];
                hex::decode_to_slice(s, &mut bytes)
                    .map_err(|_| Error::Malformed(concat!("invalid ", stringify!($name))))?;
                Ok(Self(bytes))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.to_hex())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.to_hex())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_hex())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

hex_id!(
    /// SHA-256 of a blob: its Blossom address.
    Sha256Hash,
    32
);

hex_id!(
    /// Stable identity of a file or folder, kept across edits, renames and
    /// moves. It only appears inside encrypted versions.
    NodeId,
    16
);

hex_id!(
    /// Identity of a drive for its whole life, across key rotations.
    DriveId,
    16
);

hex_id!(
    /// Random id of an install. Version vectors count each device's
    /// versions, so two installs must never share one.
    DeviceId,
    16
);

hex_id!(
    /// SHA-256 of a version's plaintext, padding included. Not the id of a
    /// gift wrap carrying it: a version can travel in several wraps.
    VersionId,
    32
);

impl Sha256Hash {
    pub fn digest(data: &[u8]) -> Self {
        Self(Sha256::digest(data).into())
    }
}

impl VersionId {
    pub fn of(json: &[u8]) -> Self {
        Self(Sha256::digest(json).into())
    }
}

impl NodeId {
    pub fn random() -> Self {
        Self(random_bytes())
    }

    /// Node of the conflict copy made from the head `losing`. Two devices
    /// resolving the same conflict create the same copy.
    pub fn conflict_copy(losing: &VersionId) -> Self {
        let digest: [u8; 32] = Sha256::new()
            .chain_update(b"lupine/v1/conflict")
            .chain_update(losing.as_bytes())
            .finalize()
            .into();
        Self(digest[..Self::LEN].try_into().expect("node id length"))
    }
}

impl DeviceId {
    pub fn random() -> Self {
        Self(random_bytes())
    }
}

impl DriveId {
    pub fn random() -> Self {
        Self(random_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let id = NodeId::random();
        assert_eq!(id.to_hex().parse::<NodeId>().unwrap(), id);
        assert!("abc".parse::<NodeId>().is_err());
        assert!(id.to_hex().parse::<Sha256Hash>().is_err());
    }

    #[test]
    fn conflict_copies_are_deterministic() {
        let losing = VersionId([1; 32]);
        assert_eq!(
            NodeId::conflict_copy(&losing),
            NodeId::conflict_copy(&losing)
        );
        assert_ne!(
            NodeId::conflict_copy(&losing),
            NodeId::conflict_copy(&VersionId([2; 32]))
        );
    }

    #[test]
    fn serde_as_hex_string() {
        let hash = Sha256Hash::digest(b"");
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(
            json,
            "\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\""
        );
        assert_eq!(serde_json::from_str::<Sha256Hash>(&json).unwrap(), hash);
    }
}

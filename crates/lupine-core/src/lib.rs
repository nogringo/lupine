//! Lupine core: the parts of the drive that must behave identically on every
//! platform. See `docs/protocol.md` for the wire formats implemented here.

pub mod blob;
pub mod content;
pub mod crypto;
pub mod drive;
mod encoding;
pub mod error;
pub mod ids;
pub mod io;
pub mod node;

pub use error::{Error, Result};

/// Provisional, see `docs/protocol.md`.
pub mod kinds {
    pub const DRIVE_DESCRIPTOR: u16 = 3540;
}

//! Lupine core: the parts of the drive that must behave identically on every
//! platform. See `docs/protocol.md` for the wire formats implemented here.

pub mod account;
pub mod blob;
pub mod blossom;
pub mod content;
pub mod crypto;
pub mod descriptor;
pub mod drive;
mod encoding;
pub mod error;
pub mod fetch;
pub mod folder_sync;
pub mod graph;
pub mod ids;
pub mod io;
pub mod items;
pub mod names;
pub mod node;
pub mod relays;
pub mod replica;
pub mod session;
pub mod store;
mod task;
#[cfg(test)]
mod testing;
pub mod tree;
pub mod writer;

pub use error::{Error, Result};

/// Provisional, see `docs/protocol.md`.
pub mod kinds {
    pub const DRIVE_DESCRIPTOR: u16 = 3540;
}

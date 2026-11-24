//! SQLite's files, in the origin private file system. A pool of sync access
//! handles holds them, which only a dedicated worker can open.

use std::cell::OnceCell;

use rsqlite_vfs::VfsFilesManager;
use sqlite_wasm_rs::WasmOsCallback;
use sqlite_wasm_vfs::sahpool::{self, OpfsSAHPoolCfgBuilder, OpfsSAHPoolUtil};
use wasm_bindgen::prelude::*;

use crate::CoreError;

/// A database and its journal, and the same for a key rotation's scratch
/// database, with room to spare.
const FILES: usize = 8;

thread_local! {
    static POOL: OnceCell<OpfsSAHPoolUtil> = const { OnceCell::new() };
}

/// Makes OPFS SQLite's default storage. Once per worker, before any
/// database opens. Fails in another tab's worker while one holds it.
#[wasm_bindgen(js_name = initStorage)]
pub async fn init_storage() -> Result<(), CoreError> {
    if POOL.with(|pool| pool.get().is_some()) {
        return Ok(());
    }
    let config = OpfsSAHPoolCfgBuilder::new()
        .directory("lupine/sqlite")
        .initial_capacity(FILES)
        .build();
    let pool = sahpool::install::<WasmOsCallback>(&config, true)
        .await
        .map_err(|e| CoreError::Core(format!("opening the local storage: {e}")))?;
    pool.ensure_capacity(FILES)
        .await
        .map_err(|e| CoreError::Core(e.to_string()))?;
    POOL.with(|cell| {
        let _ = cell.set(pool);
    });
    Ok(())
}

/// Deletes a database and what SQLite keeps beside it. Every connection to
/// it must be closed.
pub(crate) fn remove_database(path: &str) -> Result<(), CoreError> {
    POOL.with(|pool| {
        let pool = pool
            .get()
            .ok_or_else(|| CoreError::Core("the local storage is not open".into()))?;
        for suffix in ["", "-journal", "-wal", "-shm"] {
            pool.remove(&format!("{path}{suffix}"))
                .map_err(|e| CoreError::Core(e.to_string()))?;
        }
        Ok(())
    })
}

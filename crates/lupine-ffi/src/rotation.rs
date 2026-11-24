//! Key rotation: moving the drive to a new key, after the user signed in
//! with [`SignIn::prepare_rotation`](crate::account::SignIn), then retiring
//! the old key.

use std::sync::Arc;

use lupine_core::rotation::{self, Stage};

use crate::CoreError;
use crate::account::SignIn;
use crate::local::{LocalDrive, secret_keys};

#[derive(Debug, uniffi::Enum)]
pub enum RotationStage {
    Lists,
    Claiming { done: u64, total: u64 },
    Wrapping { done: u64, total: u64 },
    Sending,
    Publishing,
    Moving,
}

impl From<Stage> for RotationStage {
    fn from(stage: Stage) -> Self {
        let count = |n: usize| n as u64;
        match stage {
            Stage::Lists => Self::Lists,
            Stage::Claiming(done, total) => Self::Claiming {
                done: count(done),
                total: count(total),
            },
            Stage::Wrapping(done, total) => Self::Wrapping {
                done: count(done),
                total: count(total),
            },
            Stage::Sending => Self::Sending,
            Stage::Publishing => Self::Publishing,
            Stage::Moving => Self::Moving,
        }
    }
}

#[uniffi::export(callback_interface)]
pub trait RotationObserver: Send + Sync {
    fn progressed(&self, stage: RotationStage);
}

#[derive(Debug, uniffi::Record)]
pub struct RotationReport {
    pub wrapped: u64,
    /// Blob copies left under the old key only, on servers that do not
    /// mirror or that refused.
    pub unclaimed: u64,
    /// The new recovery key: the old one no longer opens the drive.
    pub recovery_key: String,
}

/// Moves the drive to the key of the rotation this device started, from
/// `secret`, its current key, to `next_secret`. Stop syncing the drive
/// first, and open it again after. `sign_in` publishes the descriptor as
/// the user, when still signed in. Resumes a rotation cut short.
#[uniffi::export(async_runtime = "tokio")]
pub async fn rotate_drive_key(
    drive: Arc<LocalDrive>,
    secret: String,
    next_secret: String,
    sign_in: Option<Arc<SignIn>>,
    observer: Box<dyn RotationObserver>,
) -> Result<RotationReport, CoreError> {
    let old = drive.keys(&secret)?;
    let new = secret_keys(&next_secret)?;
    let account = sign_in.as_ref().map(|s| s.account());
    let progress = |stage: Stage| observer.progressed(stage.into());
    let rotated = rotation::complete(
        &drive.store,
        &drive.scratch(),
        &old,
        &new,
        account,
        progress,
    )
    .await?;
    let recovery_key = drive.recovery_key(next_secret)?;
    Ok(RotationReport {
        wrapped: rotated.wrapped as u64,
        unclaimed: rotated.unclaimed as u64,
        recovery_key,
    })
}

/// Gives up the rotation this device started, unless it announced the new
/// key already.
#[uniffi::export(async_runtime = "tokio")]
pub async fn abandon_rotation(
    drive: Arc<LocalDrive>,
    next_secret: Option<String>,
) -> Result<(), CoreError> {
    let new = next_secret.as_deref().map(secret_keys).transpose()?;
    Ok(rotation::abandon(&drive.store, &drive.scratch(), new.as_ref()).await?)
}

/// Deletes, with the key a rotation retired, the wraps still addressed to
/// it. Returns whether the key is retired; otherwise try again later.
/// Without its secret, gives up.
#[uniffi::export(async_runtime = "tokio")]
pub async fn retire_drive_key(
    drive: Arc<LocalDrive>,
    previous_secret: Option<String>,
) -> Result<bool, CoreError> {
    let Some(previous) = previous_secret else {
        drive.store.forget_retirement()?;
        return Ok(true);
    };
    let old = secret_keys(&previous)?;
    Ok(rotation::retire(&drive.store, &old).await?)
}

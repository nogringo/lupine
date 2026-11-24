//! What the core needs from its runtime: tokio natively, the browser's event
//! loop on the web, which runs one thread.

use std::future::{Future, IntoFuture};
use std::time::Duration;

pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// Runs CPU and disk work off the async workers.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn blocking<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(value) => value,
        Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        Err(_) => panic!("blocking task cancelled by runtime shutdown"),
    }
}

#[cfg(target_family = "wasm")]
pub(crate) async fn blocking<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    f()
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(target_family = "wasm")]
pub(crate) async fn sleep(duration: Duration) {
    sendable(gloo_timers::future::sleep(duration)).await;
}

/// `None` when `duration` passes first.
pub(crate) async fn timeout<F: IntoFuture>(duration: Duration, future: F) -> Option<F::Output> {
    use futures::future::{Either, select};
    let future = std::pin::pin!(future.into_future());
    match select(future, std::pin::pin!(sleep(duration))).await {
        Either::Left((output, _)) => Some(output),
        Either::Right(_) => None,
    }
}

/// Browser futures are not `Send`, which the web's single thread makes moot.
#[cfg(target_family = "wasm")]
pub(crate) fn sendable<F: Future>(future: F) -> send_wrapper::SendWrapper<F> {
    send_wrapper::SendWrapper::new(future)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn sendable<F: Future>(future: F) -> F {
    future
}

/// Runs CPU and disk work off the async workers.
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

use std::fmt::Display;
use std::thread::JoinHandle;

pub trait Report<T> {
    fn report(self, scope: impl Display) -> Option<T>;
}

impl<T, E: Into<anyhow::Error>> Report<T> for Result<T, E> {
    fn report(self, scope: impl Display) -> Option<T> {
        self.map_err(|e| tracing::error!("{scope}: {:#}", e.into()))
            .ok()
    }
}

pub fn spawn(
    scope: impl Into<String>,
    task: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
) -> JoinHandle<()> {
    let scope = scope.into();
    std::thread::Builder::new()
        .name(scope.clone())
        .spawn(move || task().report(&scope).unwrap_or_default())
        .expect("failed to spawn thread")
}

#[derive(Debug)]
pub struct Cancelled;

impl Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn ignore_cancel(result: anyhow::Result<()>) -> anyhow::Result<()> {
    result.or_else(|e| e.downcast_ref::<Cancelled>().map(|_| ()).ok_or(e))
}

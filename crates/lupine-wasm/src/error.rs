use wasm_bindgen::{JsCast, JsValue};

/// Thrown as a JS `Error` whose `name` is the variant's, so the page can tell
/// them apart after `postMessage`.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no such item")]
    NoSuchItem,
    /// May only be late.
    #[error("no such parent folder")]
    NoSuchParent,
    #[error("the folder is not empty")]
    DirectoryNotEmpty,
    #[error("{0}")]
    InvalidName(String),
    #[error("the requested version is no longer available")]
    VersionNoLongerAvailable,
    #[error("cancelled")]
    Cancelled,
    /// May succeed later.
    #[error("{0}")]
    Unreachable(String),
    #[error("{0}")]
    Core(String),
    #[error("{0}")]
    Check(String),
}

impl CoreError {
    fn name(&self) -> &'static str {
        match self {
            Self::NoSuchItem => "NoSuchItem",
            Self::NoSuchParent => "NoSuchParent",
            Self::DirectoryNotEmpty => "DirectoryNotEmpty",
            Self::InvalidName(_) => "InvalidName",
            Self::VersionNoLongerAvailable => "VersionNoLongerAvailable",
            Self::Cancelled => "Cancelled",
            Self::Unreachable(_) => "Unreachable",
            Self::Core(_) => "Core",
            Self::Check(_) => "Check",
        }
    }
}

impl From<lupine_core::Error> for CoreError {
    fn from(e: lupine_core::Error) -> Self {
        match e {
            lupine_core::Error::UnknownNode => Self::NoSuchItem,
            lupine_core::Error::UnknownParent => Self::NoSuchParent,
            lupine_core::Error::FolderNotEmpty => Self::DirectoryNotEmpty,
            e @ lupine_core::Error::InvalidName => Self::InvalidName(e.to_string()),
            e if e.is_transient() => Self::Unreachable(e.to_string()),
            e => Self::Core(e.to_string()),
        }
    }
}

impl From<serde_wasm_bindgen::Error> for CoreError {
    fn from(e: serde_wasm_bindgen::Error) -> Self {
        Self::Check(format!("invalid argument: {e}"))
    }
}

impl From<CoreError> for JsValue {
    fn from(e: CoreError) -> Self {
        let error = js_sys::Error::new(&e.to_string());
        error.set_name(e.name());
        error.into()
    }
}

pub(crate) fn describe(error: &JsValue) -> String {
    match error.dyn_ref::<js_sys::Error>() {
        Some(error) => String::from(error.message()),
        None => error.as_string().unwrap_or_else(|| format!("{error:?}")),
    }
}

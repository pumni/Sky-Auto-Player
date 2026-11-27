//! Narrow Windows security and process-ownership boundaries used by repository tooling.

mod process_tree;
#[cfg(windows)]
mod win32;

pub mod credential;
#[cfg(windows)]
pub mod registry;

pub use process_tree::ProcessTree;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBoundaryError(String);

impl NativeBoundaryError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for NativeBoundaryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for NativeBoundaryError {}

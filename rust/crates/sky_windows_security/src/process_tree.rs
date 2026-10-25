use crate::NativeBoundaryError;
#[cfg(windows)]
use crate::win32;
use std::process::Child;

/// Owns a Windows Job Object assigned to a spawned child. Closing the owner
/// terminates every process still in the job, including descendants.
pub struct ProcessTree {
    #[cfg(windows)]
    job: win32::JobHandle,
}

impl ProcessTree {
    #[cfg(windows)]
    pub fn attach(child: &Child) -> Result<Self, NativeBoundaryError> {
        Ok(Self {
            job: win32::JobHandle::for_child(child)?,
        })
    }

    #[cfg(not(windows))]
    pub fn attach(_child: &Child) -> Result<Self, NativeBoundaryError> {
        Err(NativeBoundaryError::new(
            "Windows process-tree ownership is unavailable on this platform",
        ))
    }

    /// Requests immediate termination for every process currently owned by
    /// the job. Drop remains a kill-on-close fallback if this call fails.
    #[cfg(windows)]
    pub fn terminate(&self) -> Result<(), NativeBoundaryError> {
        self.job.terminate()
    }

    /// Keeps cross-platform callers explicit while refusing to claim that a
    /// Windows process-tree teardown occurred on other operating systems.
    #[cfg(not(windows))]
    pub fn terminate(&self) -> Result<(), NativeBoundaryError> {
        Err(NativeBoundaryError::new(
            "Windows process-tree termination is unavailable on this platform",
        ))
    }
}

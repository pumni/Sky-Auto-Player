use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdatePhase {
    Starting,
    WaitingForParent,
    FetchingRelease,
    VerifyingRelease,
    Extracting,
    VerifyingStaging,
    Preflight,
    BackingUp,
    Installing,
    VerifyingInstall,
    Committing,
    CleaningUp,
    Restarting,
    Completed,
    Failed,
    RolledBack,
}

impl UpdatePhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::WaitingForParent => "WaitingForParent",
            Self::FetchingRelease => "FetchingRelease",
            Self::VerifyingRelease => "VerifyingRelease",
            Self::Extracting => "Extracting",
            Self::VerifyingStaging => "VerifyingStaging",
            Self::Preflight => "Preflight",
            Self::BackingUp => "BackingUp",
            Self::Installing => "Installing",
            Self::VerifyingInstall => "VerifyingInstall",
            Self::Committing => "Committing",
            Self::CleaningUp => "CleaningUp",
            Self::Restarting => "Restarting",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::RolledBack => "RolledBack",
        }
    }

    pub const fn display_text(self) -> &'static str {
        match self {
            Self::Starting => "Preparing updater...",
            Self::WaitingForParent => "Waiting for Sky Auto Player to close...",
            Self::FetchingRelease => "Downloading release package...",
            Self::VerifyingRelease => "Verifying release integrity...",
            Self::Extracting => "Extracting update package...",
            Self::VerifyingStaging => "Verifying staged application files...",
            Self::Preflight => "Checking installation readiness...",
            Self::BackingUp => "Backing up current application files...",
            Self::Installing => "Installing application files...",
            Self::VerifyingInstall => "Verifying installed application...",
            Self::Committing => "Committing update...",
            Self::CleaningUp => "Cleaning temporary update files...",
            Self::Restarting => "Restarting Sky Auto Player...",
            Self::Completed => "Update complete.",
            Self::Failed => "Update could not complete.",
            Self::RolledBack => "Update could not complete. The previous version was restored.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressEvent {
    pub phase: UpdatePhase,
    pub current: Option<u64>,
    pub total: Option<u64>,
}

pub trait ProgressSink: Send + Sync {
    fn publish(&self, event: ProgressEvent);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopProgressSink;

impl ProgressSink for NoopProgressSink {
    fn publish(&self, _event: ProgressEvent) {}
}

pub type SharedProgressSink = Arc<dyn ProgressSink>;

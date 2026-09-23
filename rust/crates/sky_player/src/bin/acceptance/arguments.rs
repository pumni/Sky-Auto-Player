use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scenario {
    CanonicalSingle,
    CanonicalChord,
    CanonicalMaxChord,
    Hold,
    LongSingleSequence,
    DenseAlternating,
    ChordSweep,
    NearMinimumRetrigger,
    RapidRetrigger,
    ReleaseGapStress,
    MixedUpDown,
    AmbiguousPacket,
    PreflightUserHeld,
    ModifierHeldFinalBoundary,
    ModifierHeldAfterOwned,
    CleanupFullRelease,
    FocusLoss,
    TargetHwndChange,
    OwnerMismatch,
    OwnerQueryFailure,
    OwnerProcessTermination,
    PauseResume,
    SuspendResume,
    StopCleanup,
    SkipCleanup,
    SupervisorLeaseExpiry,
    W4Noncanonical,
    TimingMarginSweep,
}

impl Scenario {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "canonical-single" => Ok(Self::CanonicalSingle),
            "canonical-chord" => Ok(Self::CanonicalChord),
            "canonical-max-chord" => Ok(Self::CanonicalMaxChord),
            "hold" => Ok(Self::Hold),
            "long-single-sequence" => Ok(Self::LongSingleSequence),
            "dense-alternating" => Ok(Self::DenseAlternating),
            "chord-sweep" => Ok(Self::ChordSweep),
            "near-minimum-retrigger" => Ok(Self::NearMinimumRetrigger),
            "rapid-retrigger" => Ok(Self::RapidRetrigger),
            "release-gap-stress" => Ok(Self::ReleaseGapStress),
            "mixed-up-down" => Ok(Self::MixedUpDown),
            "ambiguous-packet" => Ok(Self::AmbiguousPacket),
            "preflight-user-held" => Ok(Self::PreflightUserHeld),
            "modifier-held-final-boundary" => Ok(Self::ModifierHeldFinalBoundary),
            "modifier-held-after-owned" => Ok(Self::ModifierHeldAfterOwned),
            "cleanup-full-release" => Ok(Self::CleanupFullRelease),
            "focus-loss" => Ok(Self::FocusLoss),
            "target-hwnd-change" => Ok(Self::TargetHwndChange),
            "owner-mismatch" => Ok(Self::OwnerMismatch),
            "owner-query-failure" => Ok(Self::OwnerQueryFailure),
            "owner-process-termination" => Ok(Self::OwnerProcessTermination),
            "pause-resume" => Ok(Self::PauseResume),
            "suspend-resume" => Ok(Self::SuspendResume),
            "stop-cleanup" => Ok(Self::StopCleanup),
            "skip-cleanup" => Ok(Self::SkipCleanup),
            "supervisor-lease-expiry" => Ok(Self::SupervisorLeaseExpiry),
            "w4-noncanonical" => Ok(Self::W4Noncanonical),
            "timing-margin-sweep" => Ok(Self::TimingMarginSweep),
            _ => Err(format!("unsupported scenario: {value}")),
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::CanonicalSingle => "canonical-single",
            Self::CanonicalChord => "canonical-chord",
            Self::CanonicalMaxChord => "canonical-max-chord",
            Self::Hold => "hold",
            Self::LongSingleSequence => "long-single-sequence",
            Self::DenseAlternating => "dense-alternating",
            Self::ChordSweep => "chord-sweep",
            Self::NearMinimumRetrigger => "near-minimum-retrigger",
            Self::RapidRetrigger => "rapid-retrigger",
            Self::ReleaseGapStress => "release-gap-stress",
            Self::MixedUpDown => "mixed-up-down",
            Self::AmbiguousPacket => "ambiguous-packet",
            Self::PreflightUserHeld => "preflight-user-held",
            Self::ModifierHeldFinalBoundary => "modifier-held-final-boundary",
            Self::ModifierHeldAfterOwned => "modifier-held-after-owned",
            Self::CleanupFullRelease => "cleanup-full-release",
            Self::FocusLoss => "focus-loss",
            Self::TargetHwndChange => "target-hwnd-change",
            Self::OwnerMismatch => "owner-mismatch",
            Self::OwnerQueryFailure => "owner-query-failure",
            Self::OwnerProcessTermination => "owner-process-termination",
            Self::PauseResume => "pause-resume",
            Self::SuspendResume => "suspend-resume",
            Self::StopCleanup => "stop-cleanup",
            Self::SkipCleanup => "skip-cleanup",
            Self::SupervisorLeaseExpiry => "supervisor-lease-expiry",
            Self::W4Noncanonical => "w4-noncanonical",
            Self::TimingMarginSweep => "timing-margin-sweep",
        }
    }

    pub(super) const fn needs_focus_probe(self) -> bool {
        matches!(self, Self::FocusLoss)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunArgs {
    pub(super) run_id: String,
    pub(super) sink_ready: PathBuf,
    pub(super) sink_events: PathBuf,
    pub(super) target_hwnd: isize,
    pub(super) scenario: Scenario,
    pub(super) evidence: PathBuf,
    pub(super) timing_margin_us: u64,
    pub(super) focus_probe_ready: Option<PathBuf>,
    pub(super) focus_probe_events: Option<PathBuf>,
    pub(super) focus_probe_hwnd: Option<isize>,
    pub(super) focus_restore_request: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ParsedCommand {
    Help,
    Run(Box<RunArgs>),
}

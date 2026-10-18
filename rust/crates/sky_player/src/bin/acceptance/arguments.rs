use std::path::PathBuf;

use super::{
    ACCEPTANCE_TIMING_MARGIN_MAX_US, ACCEPTANCE_TIMING_MARGIN_MIN_US,
    ACCEPTANCE_TIMING_MARGIN_STEP_US, ACCEPTANCE_TIMING_MARGIN_US, MAX_PATH_BYTES,
    MAX_RUN_ID_BYTES, usage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CalibrationPlacement {
    Baseline,
    Candidate,
}

impl CalibrationPlacement {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "candidate" => Ok(Self::Candidate),
            _ => Err("--calibration-placement must be baseline or candidate".to_string()),
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
}

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
    PauseResumeReleaseFloor,
    SuspendResume,
    StopCleanup,
    SkipCleanup,
    SupervisorLeaseExpiry,
    LeaseAgeWatchdogDelayed,
    NoncanonicalProfile,
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
            "pause-resume-release-floor" => Ok(Self::PauseResumeReleaseFloor),
            "suspend-resume" => Ok(Self::SuspendResume),
            "stop-cleanup" => Ok(Self::StopCleanup),
            "skip-cleanup" => Ok(Self::SkipCleanup),
            "supervisor-lease-expiry" => Ok(Self::SupervisorLeaseExpiry),
            "lease-age-watchdog-delayed" => Ok(Self::LeaseAgeWatchdogDelayed),
            "noncanonical-profile" => Ok(Self::NoncanonicalProfile),
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
            Self::PauseResumeReleaseFloor => "pause-resume-release-floor",
            Self::SuspendResume => "suspend-resume",
            Self::StopCleanup => "stop-cleanup",
            Self::SkipCleanup => "skip-cleanup",
            Self::SupervisorLeaseExpiry => "supervisor-lease-expiry",
            Self::LeaseAgeWatchdogDelayed => "lease-age-watchdog-delayed",
            Self::NoncanonicalProfile => "noncanonical-profile",
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
    pub(super) calibration_placement: CalibrationPlacement,
    pub(super) timing_observer_for_qualification: bool,
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

fn validate_run_id(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > MAX_RUN_ID_BYTES
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("run ID must be 1..128 non-whitespace, non-control characters".to_string());
    }
    Ok(value.to_string())
}

fn bounded_path(value: &str, flag: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.chars().any(char::is_control) {
        return Err(format!("{flag} must be a bounded nonempty path"));
    }
    Ok(PathBuf::from(value))
}

fn parse_hwnd(value: &str) -> Result<isize, String> {
    let value = value.trim();
    let parsed = if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else if let Some(hex) = value.strip_prefix("0X") {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse::<u64>()
    }
    .map_err(|_| "target HWND must be a decimal or 0x-prefixed hexadecimal integer".to_string())?;
    if parsed == 0 {
        return Err("target HWND must be nonzero".to_string());
    }
    let max = isize::MAX as u64;
    if parsed > max {
        return Err("target HWND is outside the positive isize range".to_string());
    }
    Ok(parsed as isize)
}

fn parse_timing_margin_us(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "--timing-margin-us must be 0..3000 in 100 us steps".to_string())?;
    if (ACCEPTANCE_TIMING_MARGIN_MIN_US..=ACCEPTANCE_TIMING_MARGIN_MAX_US).contains(&parsed)
        && parsed % ACCEPTANCE_TIMING_MARGIN_STEP_US == 0
    {
        Ok(parsed)
    } else {
        Err("--timing-margin-us must be 0..3000 in 100 us steps".to_string())
    }
}

pub(super) fn parse_args<I>(arguments: I) -> Result<ParsedCommand, String>
where
    I: IntoIterator<Item = String>,
{
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        return Err(usage().to_string());
    };
    if command == "--help" || command == "-h" {
        return Ok(ParsedCommand::Help);
    }
    if command != "run" {
        return Err(format!("expected subcommand run, got {command:?}"));
    }
    let mut allow_real_input = false;
    let mut run_id = None;
    let mut sink_ready = None;
    let mut sink_events = None;
    let mut target_hwnd = None;
    let mut scenario = None;
    let mut calibration_placement = None;
    let mut timing_observer_for_qualification = false;
    let mut evidence = None;
    let mut timing_margin_us = None;
    let mut focus_probe_ready = None;
    let mut focus_probe_events = None;
    let mut focus_probe_hwnd = None;
    let mut focus_restore_request = None;
    let mut arguments = arguments.peekable();
    while let Some(flag) = arguments.next() {
        macro_rules! next_value {
            ($name:literal) => {
                arguments
                    .next()
                    .ok_or_else(|| format!("{} requires a value", $name))?
            };
        }
        macro_rules! unique {
            ($slot:ident, $name:literal) => {{
                if $slot.is_some() {
                    return Err(format!("duplicate {}", $name));
                }
            }};
        }
        if flag == "--allow-real-input" {
            if allow_real_input {
                return Err("duplicate --allow-real-input".to_string());
            }
            allow_real_input = true;
            continue;
        }
        if flag == "--timing-observer-for-qualification" {
            if timing_observer_for_qualification {
                return Err("duplicate --timing-observer-for-qualification".to_string());
            }
            timing_observer_for_qualification = true;
            continue;
        }
        match flag.as_str() {
            "--run-id" => {
                unique!(run_id, "--run-id");
                run_id = Some(validate_run_id(&next_value!("--run-id"))?);
            }
            "--sink-ready" => {
                unique!(sink_ready, "--sink-ready");
                let value = next_value!("--sink-ready");
                sink_ready = Some(bounded_path(&value, "--sink-ready")?);
            }
            "--sink-events" => {
                unique!(sink_events, "--sink-events");
                let value = next_value!("--sink-events");
                sink_events = Some(bounded_path(&value, "--sink-events")?);
            }
            "--target-hwnd" => {
                unique!(target_hwnd, "--target-hwnd");
                target_hwnd = Some(parse_hwnd(&next_value!("--target-hwnd"))?);
            }
            "--scenario" => {
                unique!(scenario, "--scenario");
                scenario = Some(Scenario::parse(&next_value!("--scenario"))?);
            }
            "--calibration-placement" => {
                unique!(calibration_placement, "--calibration-placement");
                calibration_placement = Some(CalibrationPlacement::parse(&next_value!(
                    "--calibration-placement"
                ))?);
            }
            "--evidence" => {
                unique!(evidence, "--evidence");
                let value = next_value!("--evidence");
                evidence = Some(bounded_path(&value, "--evidence")?);
            }
            "--timing-margin-us" => {
                unique!(timing_margin_us, "--timing-margin-us");
                timing_margin_us = Some(parse_timing_margin_us(&next_value!(
                    "--timing-margin-us"
                ))?);
            }
            "--focus-probe-ready" => {
                unique!(focus_probe_ready, "--focus-probe-ready");
                let value = next_value!("--focus-probe-ready");
                focus_probe_ready = Some(bounded_path(&value, "--focus-probe-ready")?);
            }
            "--focus-probe-events" => {
                unique!(focus_probe_events, "--focus-probe-events");
                let value = next_value!("--focus-probe-events");
                focus_probe_events = Some(bounded_path(&value, "--focus-probe-events")?);
            }
            "--focus-probe-hwnd" => {
                unique!(focus_probe_hwnd, "--focus-probe-hwnd");
                focus_probe_hwnd = Some(parse_hwnd(&next_value!("--focus-probe-hwnd"))?);
            }
            "--focus-restore-request" => {
                unique!(focus_restore_request, "--focus-restore-request");
                let value = next_value!("--focus-restore-request");
                focus_restore_request = Some(bounded_path(&value, "--focus-restore-request")?);
            }
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            _ => return Err(format!("unknown argument {flag:?}")),
        }
    }
    if !allow_real_input {
        return Err("--allow-real-input is required".to_string());
    }
    let scenario = scenario.ok_or_else(|| "--scenario is required".to_string())?;
    let run = RunArgs {
        run_id: run_id.ok_or_else(|| "--run-id is required".to_string())?,
        sink_ready: sink_ready.ok_or_else(|| "--sink-ready is required".to_string())?,
        sink_events: sink_events.ok_or_else(|| "--sink-events is required".to_string())?,
        target_hwnd: target_hwnd.ok_or_else(|| "--target-hwnd is required".to_string())?,
        scenario,
        calibration_placement: calibration_placement.unwrap_or(CalibrationPlacement::Baseline),
        timing_observer_for_qualification,
        evidence: evidence.ok_or_else(|| "--evidence is required".to_string())?,
        timing_margin_us: timing_margin_us.unwrap_or(ACCEPTANCE_TIMING_MARGIN_US),
        focus_probe_ready,
        focus_probe_events,
        focus_probe_hwnd,
        focus_restore_request,
    };
    let probe_count = [
        run.focus_probe_ready.is_some(),
        run.focus_probe_events.is_some(),
        run.focus_probe_hwnd.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if run.scenario.needs_focus_probe() && probe_count != 3 {
        return Err(
            "focus-loss requires --focus-probe-ready, --focus-probe-events, and --focus-probe-hwnd"
                .to_string(),
        );
    }
    if run.scenario.needs_focus_probe() != run.focus_restore_request.is_some() {
        return Err(
            "focus-loss requires --focus-restore-request and other scenarios may not use it"
                .to_string(),
        );
    }
    if !run.scenario.needs_focus_probe() && probe_count != 0 {
        return Err("focus-probe arguments are valid only for focus-loss".to_string());
    }
    Ok(ParsedCommand::Run(Box::new(run)))
}

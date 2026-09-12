#![cfg(feature = "real-input-acceptance")]
#![recursion_limit = "256"]
#[rustfmt::skip]
mod acceptance {
#[path = "release_gap_stress.rs"] mod release_gap_stress;
use release_gap_stress::{attach_sink_window_provenance, production_visibility_qualification, scenario_plan as release_gap_scenario_plan};
#[cfg(test)]
use release_gap_stress::RELEASE_GAP_STRESS_CYCLES;
use serde::Deserialize;
use serde_json::{Value, json};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, MAX_KEYS};
use sky_dispatch_win32::focus::{
    WindowIdentity, focus_window_and_verify, inspect_window_identity,
};
#[cfg(test)]
use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;
use sky_dispatch_win32::input::{
    InstrumentKeyProfileSpec, FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalKey,
};
use sky_dispatch_win32::mmcss::PriorityMode;
use sky_player::adapter_support::compile_runtime_intents;
use sky_player::engine::{
    BackendConfig, DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, DispatchProfile, EngineSnapshot,
    FocusOptions, NativeDispatchSession, NativeSessionOptions, PriorityOptions, TelemetryMode,
    TelemetryOptions, TimingOptions, WaitOptions,
};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::{Duration, Instant};
const MAX_RUN_ID_BYTES: usize = 128;
const MAX_PATH_BYTES: usize = 4096;
const READY_SCHEMA_VERSION: u32 = 3;
const EVENT_SCHEMA_VERSION: u32 = 3;
const DRAIN_DEADLINE_MS: u64 = 1_000; const PRETERMINAL_DEADLINE_MS: u64 = 2_000;
const DRAIN_QUIET_MS: u64 = 100;
const DRAIN_POLL_MS: u64 = 10;
const RECEIVE_ONLY_ROLE: &str = "ReceiveOnly";
const PROBE_ROLE: &str = "InertFocusProbe";
const SINK_KIND: &str = "SkyAutoPlayer.NativeAcceptanceSink";
const PROBE_KIND: &str = "SkyAutoPlayer.NativeAcceptanceFocusProbe";
const SINK_TITLE: &str = "Sky Auto Player — Native Acceptance Sink";
const PROBE_TITLE: &str = "Sky Auto Player — Native Acceptance Focus Probe";
const SCRIPT_MARKER: &str = "native_acceptance_sink.ps1";
const INPUT_POLICY: &str = concat!("receives_only; benchmark must use ", "Send", "Input");
const REQUIRED_IMAGE_BASENAME: &str = "pwsh.exe";
const ACCEPTANCE_FPS: u64 = 60;
const ACCEPTANCE_FRAME_US: u64 = 1_000_000_u64.div_ceil(ACCEPTANCE_FPS);
const ACCEPTANCE_HOLD_FRAMES: u64 = 1;
const ACCEPTANCE_DOWN_LATE_GRACE_US: u64 = 500;
const ACCEPTANCE_TIMING_MARGIN_US: u64 = 800;
const ACCEPTANCE_TIMING_MARGIN_MIN_US: u64 = 0;
const ACCEPTANCE_TIMING_MARGIN_MAX_US: u64 = 3_000;
const ACCEPTANCE_TIMING_MARGIN_STEP_US: u64 = 100;
const ACCEPTANCE_INPUT_PATH_WARN_US: u64 = 300;
const ACCEPTANCE_FOCUS_RESTORE_GRACE_US: u64 = 100_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario { CanonicalSingle, CanonicalChord, CanonicalMaxChord, Hold, RapidRetrigger, ReleaseGapStress, MixedUpDown, CleanupFullRelease, FocusLoss, TargetHwndChange, PauseResume, StopCleanup, SkipCleanup, W4Noncanonical, TimingMarginSweep }
impl Scenario {
    fn parse(value: &str) -> Result<Self, String> {
        match value { "canonical-single" => Ok(Self::CanonicalSingle), "canonical-chord" => Ok(Self::CanonicalChord), "canonical-max-chord" => Ok(Self::CanonicalMaxChord), "hold" => Ok(Self::Hold), "rapid-retrigger" => Ok(Self::RapidRetrigger), "release-gap-stress" => Ok(Self::ReleaseGapStress), "mixed-up-down" => Ok(Self::MixedUpDown), "cleanup-full-release" => Ok(Self::CleanupFullRelease), "focus-loss" => Ok(Self::FocusLoss), "target-hwnd-change" => Ok(Self::TargetHwndChange), "pause-resume" => Ok(Self::PauseResume), "stop-cleanup" => Ok(Self::StopCleanup), "skip-cleanup" => Ok(Self::SkipCleanup), "w4-noncanonical" => Ok(Self::W4Noncanonical), "timing-margin-sweep" => Ok(Self::TimingMarginSweep), _ => Err(format!("unsupported scenario: {value}")) }
    }
    const fn label(self) -> &'static str {
        match self { Self::CanonicalSingle => "canonical-single", Self::CanonicalChord => "canonical-chord", Self::CanonicalMaxChord => "canonical-max-chord", Self::Hold => "hold", Self::RapidRetrigger => "rapid-retrigger", Self::ReleaseGapStress => "release-gap-stress", Self::MixedUpDown => "mixed-up-down", Self::CleanupFullRelease => "cleanup-full-release", Self::FocusLoss => "focus-loss", Self::TargetHwndChange => "target-hwnd-change", Self::PauseResume => "pause-resume", Self::StopCleanup => "stop-cleanup", Self::SkipCleanup => "skip-cleanup", Self::W4Noncanonical => "w4-noncanonical", Self::TimingMarginSweep => "timing-margin-sweep" }
    }
    const fn needs_focus_probe(self) -> bool {
        matches!(self, Self::FocusLoss)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct RunArgs {
    run_id: String, sink_ready: PathBuf, sink_events: PathBuf, target_hwnd: isize, scenario: Scenario,
    evidence: PathBuf, down_late_grace_us: u64, timing_margin_us: u64, focus_probe_ready: Option<PathBuf>, focus_probe_events: Option<PathBuf>, focus_probe_hwnd: Option<isize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedCommand {
    Help,
    Run(Box<RunArgs>),
}
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)] struct ReadyRecord { schema_version: u32, run_id: String, role: String, sink_kind: String, pid: u32, hwnd: i64, title: String, process: String, input_policy: String, event_log_id: String, event_schema_version: u32, process_start_time_filetime: u64 }
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)] struct EventRecord { schema_version: u32, run_id: String, role: String, event_log_id: String, sequence: u64, kind: String, scan_code: u16, extended: bool, virtual_key: i32, message: u32, observed_utc: String }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] struct LogCursor { offset: u64, sequence: u64 }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] struct PhysicalExpectation { scan_code: u16, extended: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] enum DrainMode { ExpectedEvents { allow_unpaired_cleanup_ups: bool }, ZeroEventSafety }
#[derive(Debug, Clone, PartialEq, Eq)] struct EventWindow { events: Vec<EventRecord>, complete: bool }
#[derive(Debug, Clone, PartialEq, Eq)] enum DrainResult { Pass(Vec<EventRecord>), Fail(String, Vec<EventRecord>), Inconclusive(String) }
#[derive(Debug, Clone, PartialEq, Eq)] enum PreTerminalResult { Satisfied, TrustedFailure(String), ObservationInconclusive(String), IncompleteTimeout { complete: bool } }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict { Pass, NonQualifying, Fail, Inconclusive }
impl Verdict {
    const fn label(self) -> &'static str { match self { Self::Pass => "PASS", Self::NonQualifying => "NON_QUALIFYING", Self::Fail => "FAIL", Self::Inconclusive => "INCONCLUSIVE" } }
    const fn exit_code(self) -> i32 { match self { Self::Pass => 0, Self::NonQualifying => 3, Self::Fail => 1, Self::Inconclusive => 2 } }
}
#[derive(Debug, Clone)]
struct ScenarioPlan {
    schedule: sky_dispatch_core::model::RuntimeSchedule, profile: Option<InstrumentKeyProfileSpec>,
    expected_down_slots: Vec<usize>, expected_up_slots: Vec<usize>, allow_unpaired_cleanup_ups: bool,
}
#[derive(Debug, Clone)]
struct AuthorizedTargets {
    sink: ReadyRecord,
    probe: Option<ReadyRecord>,
}
fn usage() -> &'static str {
    "Usage: rt-native-acceptance run --allow-real-input --run-id <id> --sink-ready <path> --sink-events <path> --target-hwnd <decimal|0xhex> --scenario <name> --evidence <path> [--timing-margin-us <0..3000, step 100>] [--down-late-grace-us <0..5000, step 100>] [--focus-probe-ready <path> --focus-probe-events <path> --focus-probe-hwnd <decimal|0xhex>]"
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
fn parse_down_late_grace_us(value: &str) -> Result<u64, String> { let parsed = value.parse::<u64>().map_err(|_| "--down-late-grace-us must be 0..5000 in 100 us steps".to_string())?; if parsed <= 5_000 && parsed % 100 == 0 { Ok(parsed) } else { Err("--down-late-grace-us must be 0..5000 in 100 us steps".to_string()) } }
fn parse_timing_margin_us(value: &str) -> Result<u64, String> {
    let parsed = value.parse::<u64>().map_err(|_| "--timing-margin-us must be 0..3000 in 100 us steps".to_string())?;
    if (ACCEPTANCE_TIMING_MARGIN_MIN_US..=ACCEPTANCE_TIMING_MARGIN_MAX_US).contains(&parsed)
        && parsed % ACCEPTANCE_TIMING_MARGIN_STEP_US == 0
    {
        Ok(parsed)
    } else {
        Err("--timing-margin-us must be 0..3000 in 100 us steps".to_string())
    }
}
fn parse_args<I>(arguments: I) -> Result<ParsedCommand, String>
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
    let mut evidence = None; let mut down_late_grace_us = None; let mut timing_margin_us = None;
    let mut focus_probe_ready = None;
    let mut focus_probe_events = None;
    let mut focus_probe_hwnd = None;
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
        match flag.as_str() {
            "--run-id" => { unique!(run_id, "--run-id"); run_id = Some(validate_run_id(&next_value!("--run-id"))?); }
            "--sink-ready" => { unique!(sink_ready, "--sink-ready"); let value = next_value!("--sink-ready"); sink_ready = Some(bounded_path(&value, "--sink-ready")?); }
            "--sink-events" => { unique!(sink_events, "--sink-events"); let value = next_value!("--sink-events"); sink_events = Some(bounded_path(&value, "--sink-events")?); }
            "--target-hwnd" => { unique!(target_hwnd, "--target-hwnd"); target_hwnd = Some(parse_hwnd(&next_value!("--target-hwnd"))?); }
            "--scenario" => { unique!(scenario, "--scenario"); scenario = Some(Scenario::parse(&next_value!("--scenario"))?); }
            "--evidence" => { unique!(evidence, "--evidence"); let value = next_value!("--evidence"); evidence = Some(bounded_path(&value, "--evidence")?); }
            "--down-late-grace-us" => { unique!(down_late_grace_us, "--down-late-grace-us"); down_late_grace_us = Some(parse_down_late_grace_us(&next_value!("--down-late-grace-us"))?); }
            "--timing-margin-us" => { unique!(timing_margin_us, "--timing-margin-us"); timing_margin_us = Some(parse_timing_margin_us(&next_value!("--timing-margin-us"))?); }
            "--focus-probe-ready" => { unique!(focus_probe_ready, "--focus-probe-ready"); let value = next_value!("--focus-probe-ready"); focus_probe_ready = Some(bounded_path(&value, "--focus-probe-ready")?); }
            "--focus-probe-events" => { unique!(focus_probe_events, "--focus-probe-events"); let value = next_value!("--focus-probe-events"); focus_probe_events = Some(bounded_path(&value, "--focus-probe-events")?); }
            "--focus-probe-hwnd" => { unique!(focus_probe_hwnd, "--focus-probe-hwnd"); focus_probe_hwnd = Some(parse_hwnd(&next_value!("--focus-probe-hwnd"))?); }
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
        evidence: evidence.ok_or_else(|| "--evidence is required".to_string())?,
        down_late_grace_us: down_late_grace_us.unwrap_or(ACCEPTANCE_DOWN_LATE_GRACE_US),
        timing_margin_us: timing_margin_us.unwrap_or(ACCEPTANCE_TIMING_MARGIN_US),
        focus_probe_ready,
        focus_probe_events,
        focus_probe_hwnd,
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
    if !run.scenario.needs_focus_probe() && probe_count != 0 {
        return Err("focus-probe arguments are valid only for focus-loss".to_string());
    }
    Ok(ParsedCommand::Run(Box::new(run)))
}
fn protocol_expectation(role: &str) -> Result<(&'static str, &'static str, &'static str), String> {
    match role {
        RECEIVE_ONLY_ROLE => Ok((SINK_KIND, SINK_TITLE, RECEIVE_ONLY_ROLE)),
        PROBE_ROLE => Ok((PROBE_KIND, PROBE_TITLE, PROBE_ROLE)),
        _ => Err(format!("unsupported ready-file role {role:?}")),
    }
}
fn validate_ready_record(
    record: &ReadyRecord,
    run_id: &str,
    target_hwnd: isize,
    role: &str,
) -> Result<(), String> {
    let (expected_kind, expected_title, expected_role) = protocol_expectation(role)?;
    if record.schema_version != READY_SCHEMA_VERSION || record.event_schema_version != EVENT_SCHEMA_VERSION { return Err("ready-file schema/event schema version must be 3".into()); }
    if record.run_id != run_id { return Err("ready-file run_id does not match --run-id".into()); }
    if record.role != expected_role || record.sink_kind != expected_kind || record.title != expected_title || record.process != SCRIPT_MARKER || record.input_policy != INPUT_POLICY {
        return Err("ready-file protocol markers do not match the expected project-owned role".into());
    }
    let ready_hwnd = isize::try_from(record.hwnd).map_err(|_| "ready-file HWND is outside the isize range".to_string())?;
    if ready_hwnd <= 0 || ready_hwnd != target_hwnd { return Err("ready-file HWND does not match the explicit target HWND".into()); }
    if record.pid == 0 || record.process_start_time_filetime == 0 || record.event_log_id.is_empty() || record.event_log_id.len() > MAX_RUN_ID_BYTES { return Err("ready-file process/event identity is incomplete".into()); }
    Ok(())
}
fn validate_live_identity(record: &ReadyRecord, live: &WindowIdentity) -> Result<(), String> {
    if live.hwnd != record.hwnd as isize
        || live.owner_pid != record.pid
        || live.title != record.title
        || live.process_start_time_filetime != record.process_start_time_filetime
        || live.process_image_basename != REQUIRED_IMAGE_BASENAME
    {
        return Err("live HWND identity does not match PID, title, FILETIME, or exact pwsh.exe owner".into());
    }
    Ok(())
}
fn read_ready(path: &Path) -> Result<ReadyRecord, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read ready file: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("malformed ready JSON: {error}"))
}
fn validate_target_ready(
    path: &Path,
    run_id: &str,
    target_hwnd: isize,
    role: &str,
) -> Result<ReadyRecord, String> {
    let record = read_ready(path)?;
    validate_ready_record(&record, run_id, target_hwnd, role)?;
    let live = inspect_window_identity(target_hwnd)?;
    validate_live_identity(&record, &live)?;
    Ok(record)
}
fn authorize(args: &RunArgs) -> Result<AuthorizedTargets, String> {
    if !args.sink_events.is_file() { return Err("sink event log does not exist".into()); }
    let sink = validate_target_ready(&args.sink_ready, &args.run_id, args.target_hwnd, RECEIVE_ONLY_ROLE)?;
    let probe = if args.scenario.needs_focus_probe() {
        let ready = args.focus_probe_ready.as_deref().ok_or_else(|| "focus probe ready file is missing".to_string())?;
        let events = args.focus_probe_events.as_deref().ok_or_else(|| "focus probe event log is missing".to_string())?;
        if !events.is_file() { return Err("focus probe event log does not exist".into()); }
        let hwnd = args.focus_probe_hwnd.ok_or_else(|| "focus probe HWND is missing".to_string())?;
        Some(validate_target_ready(ready, &args.run_id, hwnd, PROBE_ROLE)?)
    } else {
        None
    };
    Ok(AuthorizedTargets { sink, probe })
}
fn action(
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    slots: &[usize],
) -> KeyActionInput {
    let scan_codes = slots
        .iter()
        .map(|slot| PHYSICAL_INSTRUMENT_SCAN_CODES[*slot])
        .collect::<SmallVec<[u16; 4]>>();
    KeyActionInput {
        source_action_index,
        kind,
        scheduled_us,
        scan_codes,
        reason: Arc::<str>::from("rt-native-acceptance"),
    }
}
fn w4_profile_spec() -> InstrumentKeyProfileSpec {
    let mut spec = InstrumentKeyProfileSpec::canonical();
    spec.keys[0] = PhysicalKey {
        scan_code: 0x02,
        extended: false,
    };
    spec
}
fn scenario_plan(scenario: Scenario, timing_margin_us: u64) -> Result<ScenarioPlan, String> {
    let (actions, profile, expected_down_slots, expected_up_slots, allow_unpaired_cleanup_ups) =
        match scenario {
            Scenario::CanonicalSingle | Scenario::W4Noncanonical => (vec![action(0, ActionKind::Down, 50_000, &[0]), action(1, ActionKind::Up, 80_000, &[0])], (scenario == Scenario::W4Noncanonical).then_some(w4_profile_spec()), vec![0], vec![0], false),
            Scenario::CanonicalChord => (vec![action(0, ActionKind::Down, 50_000, &[0, 1]), action(1, ActionKind::Up, 90_000, &[0, 1])], None, vec![0, 1], vec![0, 1], false),
            Scenario::CanonicalMaxChord => (vec![action(0, ActionKind::Down, 50_000, &(0..MAX_KEYS).collect::<Vec<_>>()), action(1, ActionKind::Up, 90_000, &(0..MAX_KEYS).collect::<Vec<_>>())], None, (0..MAX_KEYS).collect(), (0..MAX_KEYS).collect(), false),
            Scenario::Hold => (vec![action(0, ActionKind::Down, 50_000, &[0]), action(1, ActionKind::Up, 500_000, &[0])], None, vec![0], vec![0], false),
            Scenario::RapidRetrigger => (vec![action(0, ActionKind::Down, 50_000, &[0]), action(1, ActionKind::Up, 80_000, &[0]), action(2, ActionKind::Down, 110_000, &[0]), action(3, ActionKind::Up, 140_000, &[0]), action(4, ActionKind::Down, 170_000, &[0]), action(5, ActionKind::Up, 200_000, &[0])], None, vec![0, 0, 0], vec![0, 0, 0], false),
            Scenario::ReleaseGapStress => return release_gap_scenario_plan(timing_margin_us),
            Scenario::MixedUpDown => (vec![action(0, ActionKind::Down, 50_000, &[0]), action(1, ActionKind::Up, 100_000, &[0]), action(2, ActionKind::Down, 100_000, &[1]), action(3, ActionKind::Up, 150_000, &[1])], None, vec![0, 1], vec![0, 1], false),
            Scenario::CleanupFullRelease => (vec![action(0, ActionKind::Down, 50_000, &(0..MAX_KEYS).collect::<Vec<_>>()), action(1, ActionKind::Up, 10_000_000, &(0..MAX_KEYS).collect::<Vec<_>>())], None, (0..MAX_KEYS).collect(), (0..MAX_KEYS).collect(), false),
            Scenario::FocusLoss => (vec![action(0, ActionKind::Down, 500_000, &[0]), action(1, ActionKind::Up, 600_000, &[0]), action(2, ActionKind::Down, 1_000_000, &[1]), action(3, ActionKind::Up, 1_100_000, &[1])], None, vec![0], vec![0], false),
            Scenario::TargetHwndChange => (vec![action(0, ActionKind::Down, 500_000, &[0]), action(1, ActionKind::Up, 600_000, &[0])], None, vec![], (0..MAX_KEYS).collect(), true),
            Scenario::StopCleanup | Scenario::SkipCleanup => (vec![action(0, ActionKind::Down, 50_000, &[0]), action(1, ActionKind::Up, 10_000_000, &[0])], None, vec![0], vec![0], false),
            Scenario::PauseResume => (vec![action(0, ActionKind::Down, 50_000, &[0]), action(1, ActionKind::Up, 120_000, &[0]), action(2, ActionKind::Down, 500_000, &[1]), action(3, ActionKind::Up, 570_000, &[1])], None, vec![0, 1], vec![0, 1].into_iter().chain(0..MAX_KEYS).collect(), true),
            Scenario::TimingMarginSweep => {
                let down = 50_000;
                let up = down + acceptance_min_hold_us(timing_margin_us);
                let next_down = up + acceptance_min_release_gap_us(timing_margin_us);
                let next_up = next_down + acceptance_min_hold_us(timing_margin_us);
                (vec![action(0, ActionKind::Down, down, &[0]), action(1, ActionKind::Up, up, &[0]), action(2, ActionKind::Down, next_down, &[0]), action(3, ActionKind::Up, next_up, &[0])], None, vec![0, 0], vec![0, 0], false)
            }
        };
    let schedule = compile_runtime_intents(&actions, &PHYSICAL_INSTRUMENT_SCAN_CODES)
        .map_err(|error| format!("scenario schedule compilation failed: {error}"))?;
    Ok(ScenarioPlan {
        schedule,
        profile,
        expected_down_slots,
        expected_up_slots,
        allow_unpaired_cleanup_ups,
    })
}
fn acceptance_min_hold_us(timing_margin_us: u64) -> u64 { ACCEPTANCE_HOLD_FRAMES * ACCEPTANCE_FRAME_US + timing_margin_us }
fn acceptance_min_release_gap_us(timing_margin_us: u64) -> u64 { ACCEPTANCE_FRAME_US + timing_margin_us }
fn production_options(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    profile: Option<InstrumentKeyProfileSpec>,
    down_late_grace_us: u64,
    timing_margin_us: u64,
) -> NativeSessionOptions {
    NativeSessionOptions {
        schedule,
        backend: BackendConfig::Production,
        profile: DispatchProfile::Production,
        timing: TimingOptions {
            game_fps: 60,
            min_hold_us: acceptance_min_hold_us(timing_margin_us),
            min_release_gap_us: acceptance_min_release_gap_us(timing_margin_us),
            down_late_grace_us,
            strict_timing: false,
            strict_down_completion_late_us: 2_000,
            strict_up_completion_late_us: 2_000,
            input_path_warn_us: ACCEPTANCE_INPUT_PATH_WARN_US,
        },
        focus: FocusOptions {
            require_focus: true,
            focus_restore_grace_us: ACCEPTANCE_FOCUS_RESTORE_GRACE_US,
        },
        wait: WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
            #[cfg(feature = "test-support")]
            test_spin_threshold_us: None,
            #[cfg(feature = "test-support")]
            test_wait_policy: sky_player::engine::TestWaitPolicy::ProductionCalibrated,
        },
        telemetry: TelemetryOptions {
            mode: TelemetryMode::Ring,
            capacity: 1_024,
        },
        priority: PriorityOptions {
            mode: PriorityMode::Auto,
        },
        instrument_key_profile: profile,
        #[cfg(feature = "test-support")]
        startup_ordering_hook: None,
        #[cfg(feature = "test-support")]
        restore_race_hook: None,
        #[cfg(feature = "test-support")]
        timer_lifecycle_context: None,
    }
}
fn expected_physical_keys(
    profile: Option<&InstrumentKeyProfileSpec>,
    slots: &[usize],
) -> Vec<PhysicalExpectation> {
    slots.iter().map(|slot| { let key = profile.map(|profile| profile.keys[*slot]).unwrap_or(PhysicalKey { scan_code: PHYSICAL_INSTRUMENT_SCAN_CODES[*slot], extended: false }); PhysicalExpectation { scan_code: key.scan_code, extended: key.extended } }).collect()
}
fn parse_event_log(bytes: &[u8], context: &str) -> Result<Vec<EventRecord>, String> {
    if !bytes.is_empty() && !bytes.ends_with(b"\n") { return Err(format!("{context} does not end at a complete JSONL record")); }
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| format!("{context} is not valid UTF-8"))?;
    text.lines().filter(|line| !line.is_empty()).map(|line| serde_json::from_str(line).map_err(|error| format!("{context} contains non-schema-v3 JSON: {error}"))).collect()
}
fn validate_event_stream(records: &[EventRecord], ready: &ReadyRecord, role: &str) -> Result<u64, String> {
    if ready.schema_version != READY_SCHEMA_VERSION || ready.event_schema_version != EVENT_SCHEMA_VERSION { return Err("ready-file schema/event schema version must be 3".into()); }
    let mut expected_sequence = 0_u64;
    for event in records {
        if event.schema_version != EVENT_SCHEMA_VERSION || event.run_id != ready.run_id || event.role != role || event.event_log_id != ready.event_log_id { return Err("event log is not bound to the ready-file process stream".into()); }
        if event.sequence != expected_sequence { return Err("event log sequence has a duplicate or gap".into()); }
        if expected_sequence == 0 { if event.kind != "stream_start" { return Err("event log binding header is missing".into()); } } else if event.kind == "stream_start" { return Err("event log has a duplicate/start header".into()); }
        expected_sequence = expected_sequence.saturating_add(1);
    }
    if expected_sequence == 0 { return Err("event log binding header is missing".into()); }
    Ok(expected_sequence - 1)
}
fn validate_log_binding(path: &Path, ready: &ReadyRecord, role: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read event log: {error}"))?;
    let records = parse_event_log(&bytes, "event log")?;
    validate_event_stream(&records, ready, role).map(|_| ())
}
fn read_log_cursor(path: &Path, ready: &ReadyRecord, role: &str) -> Result<LogCursor, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read event log: {error}"))?;
    let records = parse_event_log(&bytes, "event log")?;
    let sequence = validate_event_stream(&records, ready, role)?;
    Ok(LogCursor { offset: bytes.len() as u64, sequence })
}
fn read_log_window_status(
    path: &Path,
    cursor: LogCursor,
    ready: &ReadyRecord,
    role: &str,
) -> Result<EventWindow, String> {
    let all = fs::read(path).map_err(|error| format!("failed to read event log window: {error}"))?;
    let all_complete_len = if all.ends_with(b"\n") { all.len() } else { all.iter().rposition(|byte| *byte == b'\n').map_or(0, |index| index + 1) };
    let prefix = parse_event_log(&all[..all_complete_len], "event log window prefix")?;
    let prefix_sequence = validate_event_stream(&prefix, ready, role)?;
    if all.len() < cursor.offset as usize || prefix_sequence < cursor.sequence { return Err("event log was truncated or rebound after authorization".into()); }
    let bytes = all.get(cursor.offset as usize..).ok_or_else(|| "event log was truncated after authorization".to_string())?;
    let complete = bytes.is_empty() || bytes.ends_with(b"\n");
    let complete_len = if complete {
        bytes.len()
    } else {
        bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |index| index + 1)
    };
    let text = String::from_utf8(bytes[..complete_len].to_vec()).map_err(|_| "event log window is not UTF-8".to_string())?;
    let mut previous_sequence = cursor.sequence;
    let mut events = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let event: EventRecord = serde_json::from_str(line).map_err(|error| format!("event log window contains malformed JSON: {error}"))?;
        if event.run_id != ready.run_id || event.role != role || event.event_log_id != ready.event_log_id || event.schema_version != EVENT_SCHEMA_VERSION { return Err("event log window is not bound to the authorized process stream".into()); }
        if event.sequence != previous_sequence.saturating_add(1) { return Err("event log sequence has a duplicate or gap".to_string()); }
        previous_sequence = event.sequence;
        if event.kind != "stream_start" { events.push(event); }
    }
    Ok(EventWindow { events, complete })
}
fn physical_event(event: &EventRecord) -> PhysicalExpectation {
    PhysicalExpectation { scan_code: event.scan_code, extended: event.extended }
}
fn validate_event_prefix(
    events: &[EventRecord],
    expected_down: &[PhysicalExpectation],
    expected_up: &[PhysicalExpectation],
    allow_unpaired_cleanup_ups: bool,
) -> Result<bool, String> {
    let mut remaining_down = HashMap::new();
    let mut remaining_up = HashMap::new();
    for physical in expected_down { *remaining_down.entry(*physical).or_insert(0_usize) += 1; }
    for physical in expected_up { *remaining_up.entry(*physical).or_insert(0_usize) += 1; }
    let mut observed_down = HashMap::new();
    let mut observed_up = HashMap::new();
    for event in events { let physical = physical_event(event); match event.kind.as_str() {
        "key_press" => { let Some(remaining) = remaining_down.get_mut(&physical) else { return Err(format!("unexpected or duplicate KeyDown scan=0x{:02X} extended={}", physical.scan_code, physical.extended)); }; if *remaining == 0 { return Err(format!("unexpected or duplicate KeyDown scan=0x{:02X} extended={}", physical.scan_code, physical.extended)); } *remaining -= 1; *observed_down.entry(physical).or_insert(0_usize) += 1; },
        "key_release" => { let Some(remaining) = remaining_up.get_mut(&physical) else { return Err(format!("unexpected or duplicate KeyUp scan=0x{:02X} extended={}", physical.scan_code, physical.extended)); }; if *remaining == 0 { return Err(format!("unexpected or duplicate KeyUp scan=0x{:02X} extended={}", physical.scan_code, physical.extended)); }; if !allow_unpaired_cleanup_ups && observed_down.get(&physical).copied().unwrap_or(0) <= observed_up.get(&physical).copied().unwrap_or(0) { return Err(format!("KeyUp arrived before its matching KeyDown scan=0x{:02X} extended={}", physical.scan_code, physical.extended)); }; *remaining -= 1; *observed_up.entry(physical).or_insert(0_usize) += 1; },
        "sys_key_press" | "sys_key_release" => return Err("SYSKEY event cannot satisfy gameplay evidence".into()),
        other => return Err(format!("unexpected event kind {other:?}")),
    } }
    Ok(remaining_down.values().all(|count| *count == 0) && remaining_up.values().all(|count| *count == 0))
}
fn reconcile_events(
    events: &[EventRecord],
    expected_down: &[PhysicalExpectation],
    expected_up: &[PhysicalExpectation],
    allow_unpaired_cleanup_ups: bool,
) -> Result<(), String> {
    if validate_event_prefix(events, expected_down, expected_up, allow_unpaired_cleanup_ups)? {
        Ok(())
    } else {
        Err("event log is missing an expected physical KeyDown or KeyUp".into())
    }
}
fn reconcile_event_sequence(events: &[EventRecord], expected: &[(&str, PhysicalExpectation)]) -> Result<(), String> {
    if events.len() != expected.len() { return Err(format!("event count {} does not match expected {}", events.len(), expected.len())); }
    for (index, (event, (kind, physical))) in events.iter().zip(expected).enumerate() { if event.kind != *kind || physical_event(event) != *physical { return Err(format!("event {index} does not match expected {kind} scan=0x{:02X}", physical.scan_code)); } }
    Ok(())
}
trait EventWindowReader {
    fn read(&mut self) -> Result<EventWindow, String>;
}
trait DrainClock {
    fn elapsed_ms(&self) -> u64;
    fn wait_poll(&mut self);
}
fn drain_event_window_with<R: EventWindowReader, C: DrainClock>(
    reader: &mut R,
    clock: &mut C,
    mode: DrainMode,
    expected_down: &[PhysicalExpectation],
    expected_up: &[PhysicalExpectation],
) -> DrainResult {
    let mut last_events = Vec::new();
    let mut stable_since = None;
    loop {
        let now = clock.elapsed_ms();
        let sample = match reader.read() { Ok(sample) => sample, Err(error) => return DrainResult::Inconclusive(error) };
        if sample.events != last_events { last_events = sample.events.clone(); stable_since = None; }

        match mode {
            DrainMode::ZeroEventSafety => {
                if !sample.events.is_empty() { return DrainResult::Fail("zero-event safety observed a keyboard event in the bound probe stream".into(), sample.events); }
                if now >= DRAIN_DEADLINE_MS { return if sample.complete { DrainResult::Pass(sample.events) } else { DrainResult::Inconclusive("zero-event stream remained truncated at the full deadline".into()) }; }
            }
            DrainMode::ExpectedEvents { allow_unpaired_cleanup_ups } => {
                let complete = match validate_event_prefix(&sample.events, expected_down, expected_up, allow_unpaired_cleanup_ups) { Ok(complete) => complete, Err(error) => return DrainResult::Fail(error, sample.events) };
                if complete && sample.complete {
                    let since = stable_since.get_or_insert(now);
                    if now.saturating_sub(*since) >= DRAIN_QUIET_MS { return DrainResult::Pass(sample.events); }
                } else {
                    stable_since = None;
                }
                if now >= DRAIN_DEADLINE_MS { return if sample.complete { DrainResult::Fail("expected physical evidence did not settle before the drain deadline".into(), sample.events) } else { DrainResult::Inconclusive("event stream remained truncated at the drain deadline".into()) }; }
            }
        }
        clock.wait_poll();
    }
}
fn wait_for_sink_events_with<R: EventWindowReader, C: DrainClock>(reader: &mut R, clock: &mut C, expected_down: &[PhysicalExpectation], expected_up: &[PhysicalExpectation]) -> PreTerminalResult { loop { let now = clock.elapsed_ms(); let sample = match reader.read() { Ok(sample) => sample, Err(error) => return PreTerminalResult::ObservationInconclusive(error) }; match validate_event_prefix(&sample.events, expected_down, expected_up, false) { Ok(true) if sample.complete => return PreTerminalResult::Satisfied, Ok(_) => {}, Err(error) => return PreTerminalResult::TrustedFailure(error) } if now >= PRETERMINAL_DEADLINE_MS { return PreTerminalResult::IncompleteTimeout { complete: sample.complete }; } clock.wait_poll(); } }
#[cfg(windows)]
struct FileEventWindowReader<'a> { path: &'a Path, cursor: LogCursor, ready: &'a ReadyRecord, role: &'a str }
#[cfg(windows)]
impl EventWindowReader for FileEventWindowReader<'_> { fn read(&mut self) -> Result<EventWindow, String> { read_log_window_status(self.path, self.cursor, self.ready, self.role) } }
#[cfg(windows)]
struct RealDrainClock { started: Instant, poll_ms: u64 }
#[cfg(windows)]
impl DrainClock for RealDrainClock {
    fn elapsed_ms(&self) -> u64 { self.started.elapsed().as_millis() as u64 }
    fn wait_poll(&mut self) { thread::sleep(Duration::from_millis(self.poll_ms)); }
}
#[cfg(windows)]
fn drain_event_window(
    path: &Path,
    cursor: LogCursor,
    ready: &ReadyRecord,
    role: &str,
    mode: DrainMode,
    expected_down: &[PhysicalExpectation],
    expected_up: &[PhysicalExpectation],
) -> DrainResult {
    let mut reader = FileEventWindowReader { path, cursor, ready, role }; let mut clock = RealDrainClock { started: Instant::now(), poll_ms: DRAIN_POLL_MS };
    drain_event_window_with(&mut reader, &mut clock, mode, expected_down, expected_up)
}
fn cleanup_evidence_clean(full_mask_required: bool, attempted_mask: u16, attempts: u8, released: bool, stuck_mask: u16, verification_inconclusive: bool, transport_anomaly: bool) -> bool {
    released && stuck_mask == 0 && !verification_inconclusive && !transport_anomaly && (!full_mask_required || (attempted_mask == FULL_INSTRUMENT_MASK && attempts >= 1))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] struct NativeCleanupEvidence { terminal_error: bool, partial_events: u64, zero_progress_events: u64, active_count: usize, possibly_active_count: usize, failed_release_count: usize, release_failed: bool, stuck_mask: u16, verification_inconclusive: bool, transport_anomaly: bool }
impl NativeCleanupEvidence { fn is_anomalous(self) -> bool { self.terminal_error || self.partial_events != 0 || self.zero_progress_events != 0 || self.active_count != 0 || self.possibly_active_count != 0 || self.failed_release_count != 0 || self.release_failed || self.stuck_mask != 0 || self.verification_inconclusive || self.transport_anomaly } }
fn preterminal_verdict(observer: Verdict, cleanup_anomaly: bool) -> Verdict { if cleanup_anomaly { Verdict::Fail } else { observer } }
fn focus_evidence_clean(paused: bool, final_gate_focus_losses: u64, target_changes: u64, sink_events_clean: bool, probe_events_empty: bool) -> bool {
    paused && final_gate_focus_losses >= 1 && target_changes == 0 && sink_events_clean && probe_events_empty
}
fn target_change_preflight_error(snapshot: &EngineSnapshot) -> bool { snapshot.outcome.as_deref() == Some("error") && snapshot.terminal_error.as_deref().is_some_and(|error| error.contains("instrument key preflight failed")) }
fn target_change_cleanup_exception(snapshot: &EngineSnapshot) -> bool { target_change_preflight_error(snapshot) && snapshot.keys_inserted_before_failure == 0 && snapshot.active_count == 0 && snapshot.possibly_active_count == 0 && snapshot.sendinput_partial_events == 0 && snapshot.sendinput_zero_progress_failures == 0 && snapshot.release_outcome.as_ref().is_some_and(|outcome| outcome.verification_inconclusive && !outcome.transport_anomaly) }
fn snapshot_json(snapshot: &EngineSnapshot) -> Value {
    let stuck_keys = snapshot.release_outcome.as_ref().map_or(0, |outcome| u64::from(outcome.stuck_mask.count_ones()));
    let release = snapshot.release_outcome.as_ref().map(|outcome| json!({"attempted_mask": outcome.attempted_mask, "transport_anomaly": outcome.transport_anomaly, "released_successfully": outcome.released_successfully, "stuck_mask": outcome.stuck_mask, "verification_inconclusive": outcome.verification_inconclusive, "attempts": outcome.attempts}));
    json!({"status": snapshot.status, "outcome": snapshot.outcome, "last_error": snapshot.last_error, "active_count": snapshot.active_count, "possibly_active_count": snapshot.possibly_active_count, "failed_release_count": snapshot.failed_release_count, "keys_inserted_before_failure": snapshot.keys_inserted_before_failure, "stuck_keys": stuck_keys, "terminal_error": snapshot.terminal_error, "keys_dropped": snapshot.keys_dropped, "chord_split_events": snapshot.chord_split_events, "sendinput_partial_events": snapshot.sendinput_partial_events, "sendinput_zero_progress_failures": snapshot.sendinput_zero_progress_failures, "max_sendinput_pre_call_lateness_us": snapshot.max_sendinput_pre_call_lateness_us, "pre_call_lt_250us": snapshot.pre_call_lt_250us, "pre_call_250_500us": snapshot.pre_call_250_500us, "pre_call_500_750us": snapshot.pre_call_500_750us, "pre_call_750_1000us": snapshot.pre_call_750_1000us, "pre_call_1000_1500us": snapshot.pre_call_1000_1500us, "pre_call_1500_2000us": snapshot.pre_call_1500_2000us, "pre_call_ge_2000us": snapshot.pre_call_ge_2000us, "missed_down_boundaries": snapshot.missed_down_boundaries, "missed_down_keys": snapshot.missed_down_keys, "missed_backlog_boundaries": snapshot.missed_backlog_boundaries, "missed_hard_late_boundaries": snapshot.missed_hard_late_boundaries, "final_gate_cutoff_misses": snapshot.final_gate_cutoff_misses, "final_gate_focus_losses": snapshot.final_gate_focus_losses, "final_gate_target_changes": snapshot.final_gate_target_changes, "final_gate_lease_expirations": snapshot.final_gate_lease_expirations, "production_forensics_available": snapshot.production_forensics_available, "production_forensics_version": snapshot.production_forensics_version, "production_hold_pair_samples": snapshot.production_hold_pair_samples, "production_min_pre_call_hold_ticks": snapshot.production_min_pre_call_hold_ticks, "production_min_completion_hold_ticks": snapshot.production_min_completion_hold_ticks, "production_max_pre_call_shrink_ticks": snapshot.production_max_pre_call_shrink_ticks, "production_max_completion_shrink_ticks": snapshot.production_max_completion_shrink_ticks, "production_completion_hold_below_frame_count": snapshot.production_completion_hold_below_frame_count, "production_release_gap_samples": snapshot.production_release_gap_samples, "production_min_release_gap_ticks": snapshot.production_min_release_gap_ticks, "production_release_gap_below_policy_count": snapshot.production_release_gap_below_policy_count, "production_same_call_same_key_retrigger_count": snapshot.production_same_call_same_key_retrigger_count, "production_anchor_overwrite_count": snapshot.production_anchor_overwrite_count, "production_unmatched_up_count": snapshot.production_unmatched_up_count, "production_forensics_anomaly_count": snapshot.production_forensics_anomaly_count, "release_outcome": release})
}
fn write_report(args: &RunArgs, verdict: Verdict, reason: &str, details: Value) -> i32 {
    let report = json!({"status": verdict.label(), "scenario": args.scenario.label(), "run_id": args.run_id, "down_late_grace_us": args.down_late_grace_us, "timing_margin_us": args.timing_margin_us, "min_hold_us": acceptance_min_hold_us(args.timing_margin_us), "min_release_gap_us": acceptance_min_release_gap_us(args.timing_margin_us), "reason": reason, "details": details});
    let serialized = serde_json::to_string(&report).unwrap_or_else(|_| format!(r#"{{"status":"{}","reason":"report serialization failed"}}"#, verdict.label()));
    println!("{serialized}");
    if let Err(error) = append_json_line(&args.evidence, &serialized) {
        eprintln!("failed to write evidence: {error}");
        return Verdict::Inconclusive.exit_code();
    }
    verdict.exit_code()
}
fn append_json_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}
#[cfg(windows)]
fn wait_for_startup_ready(session: &NativeDispatchSession) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot();
        if snapshot.startup_ready { return true; }
        if snapshot.is_finished || Instant::now() >= deadline { return false; }
        thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(windows)]
fn wait_for_focus_pause(session: &NativeDispatchSession) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let state = session.poll_state();
        if state.is_paused { return true; }
        if state.is_finished || Instant::now() >= deadline { return false; }
        thread::sleep(Duration::from_millis(5));
    }
}
impl PreTerminalResult {
    fn verdict(&self) -> Verdict { match self { Self::Satisfied => Verdict::Pass, Self::TrustedFailure(_) | Self::IncompleteTimeout { complete: true } => Verdict::Fail, Self::ObservationInconclusive(_) | Self::IncompleteTimeout { complete: false } => Verdict::Inconclusive } }
    fn reason(&self) -> &str { match self { Self::Satisfied => "pre-terminal physical evidence is complete", Self::TrustedFailure(reason) | Self::ObservationInconclusive(reason) => reason, Self::IncompleteTimeout { complete: true } => "expected physical evidence did not arrive before the pre-terminal deadline", Self::IncompleteTimeout { complete: false } => "event stream remained truncated at the pre-terminal deadline" } }
}
#[cfg(windows)]
fn wait_for_sink_events(path: &Path, cursor: LogCursor, ready: &ReadyRecord, expected_down: &[PhysicalExpectation], expected_up: &[PhysicalExpectation]) -> PreTerminalResult { let mut reader = FileEventWindowReader { path, cursor, ready, role: RECEIVE_ONLY_ROLE }; let mut clock = RealDrainClock { started: Instant::now(), poll_ms: DRAIN_POLL_MS / 2 }; wait_for_sink_events_with(&mut reader, &mut clock, expected_down, expected_up) }
#[cfg(windows)]
fn finish_preterminal(args: &RunArgs, session: &NativeDispatchSession, result: PreTerminalResult) -> Option<i32> { if matches!(result, PreTerminalResult::Satisfied) { return None; } let observer_verdict = result.verdict(); let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); let snapshot = session.snapshot(); let release = snapshot.release_outcome.as_ref(); let evidence = NativeCleanupEvidence { terminal_error: snapshot.terminal_error.is_some(), partial_events: snapshot.sendinput_partial_events, zero_progress_events: snapshot.sendinput_zero_progress_failures, active_count: snapshot.active_count, possibly_active_count: snapshot.possibly_active_count, failed_release_count: snapshot.failed_release_count, release_failed: release.is_some_and(|outcome| !outcome.released_successfully), stuck_mask: release.map_or(0, |outcome| outcome.stuck_mask), verification_inconclusive: release.is_some_and(|outcome| outcome.verification_inconclusive), transport_anomaly: release.is_some_and(|outcome| outcome.transport_anomaly) }; let cleanup_anomaly = evidence.is_anomalous(); let verdict = preterminal_verdict(observer_verdict, cleanup_anomaly); let reason = if cleanup_anomaly && observer_verdict != Verdict::Fail { "native terminal cleanup/residue evidence is not clean".to_string() } else { result.reason().to_string() }; Some(write_report(args, verdict, &reason, snapshot_json(&snapshot))) }
#[cfg(windows)]
fn run_windows(args: RunArgs) -> i32 {
    macro_rules! inconclusive {
        ($reason:expr, $details:expr) => {
            return write_report(&args, Verdict::Inconclusive, $reason, $details)
        };
    }
    let targets = match authorize(&args) {
        Ok(targets) => targets,
        Err(error) => inconclusive!(&error, json!({})),
    };
    let sink_hwnd = targets.sink.hwnd as isize;
    if let Err(error) = validate_log_binding(&args.sink_events, &targets.sink, RECEIVE_ONLY_ROLE) { inconclusive!(&error, json!({})); }
    let probe_cursor = match targets.probe.as_ref() {
        Some(probe) => match read_log_cursor(args.focus_probe_events.as_deref().expect("focus probe event path"), probe, PROBE_ROLE) {
            Ok(cursor) => Some(cursor),
            Err(error) => inconclusive!(&error, json!({})),
        },
        None => None,
    };
    if !focus_window_and_verify(sink_hwnd, Duration::from_millis(250)) { inconclusive!("receive-only sink could not be made the exact foreground HWND", json!({})); }
    let plan = match scenario_plan(args.scenario, args.timing_margin_us) {
        Ok(plan) => plan,
        Err(error) => return write_report(&args, Verdict::Fail, &error, json!({})),
    };
    let expected_down = expected_physical_keys(plan.profile.as_ref(), &plan.expected_down_slots);
    let expected_up = expected_physical_keys(plan.profile.as_ref(), &plan.expected_up_slots);
    let authored_packet_targets = plan.schedule.packets.iter().map(|packet| json!({"scheduled_us": packet.scheduled_us, "up_mask": packet.up_mask, "down_mask": packet.down_mask})).collect::<Vec<_>>();
    let session = match NativeDispatchSession::new(production_options(plan.schedule, plan.profile, args.down_late_grace_us, args.timing_margin_us)) { Ok(session) => Arc::new(session), Err(error) => inconclusive!(&error, json!({})) };
    session.set_target_hwnd(sink_hwnd);
    session.set_focus_hint(true);
    let fresh_sink = match validate_target_ready(&args.sink_ready, &args.run_id, sink_hwnd, RECEIVE_ONLY_ROLE) {
        Ok(record) => record,
        Err(error) => inconclusive!(&error, json!({})),
    };
    if fresh_sink.event_log_id != targets.sink.event_log_id { inconclusive!("sink ready identity changed before arm", json!({})); }
    let sink_cursor = match read_log_cursor(&args.sink_events, &fresh_sink, RECEIVE_ONLY_ROLE) {
        Ok(cursor) => cursor,
        Err(error) => inconclusive!(&error, json!({})),
    };
    if let Err(error) = session.arm(0) {
        inconclusive!(&error, json!({}));
    }
    if args.scenario == Scenario::ReleaseGapStress
        && let Err(error) = release_gap_stress::start_heartbeat(Arc::clone(&session))
    {
        inconclusive!(&error, json!({}));
    }
    let mut final_probe = targets.probe.clone();
    let (mut pause_observed, mut resume_requested, mut target_changed, mut stop_requested, mut skip_requested, mut first_physical_commit_observed) = (false, false, false, false, false, false);
    let focus_gate_observed = if args.scenario.needs_focus_probe() {
        if !wait_for_startup_ready(&session) {
            let _ = session.quit();
            let _ = session.join(Duration::from_secs(5));
            inconclusive!("production session did not reach startup_ready before focus challenge", json!({}));
        }
        if let Some(code) = finish_preterminal(&args, &session, wait_for_sink_events(&args.sink_events, sink_cursor, &fresh_sink, &expected_down, &expected_up)) { return code; }
        let probe_hwnd = targets.probe.as_ref().expect("focus scenario probe").hwnd as isize;
        if !focus_window_and_verify(probe_hwnd, Duration::from_millis(250)) {
            let _ = session.quit();
            let _ = session.join(Duration::from_secs(5));
            inconclusive!("focus probe could not become the exact foreground HWND", json!({}));
        }
        let probe = targets.probe.as_ref().expect("focus scenario probe");
        let fresh_probe = match validate_target_ready(args.focus_probe_ready.as_deref().expect("focus probe ready path"), &args.run_id, probe_hwnd, PROBE_ROLE) {
            Ok(record) => record,
            Err(error) => { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!(&error, json!({})); }
        };
        if fresh_probe.event_log_id != probe.event_log_id { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!("focus probe ready identity changed after focus transition", json!({})); }
        if let Err(error) = validate_log_binding(args.focus_probe_events.as_deref().expect("focus probe event path"), &fresh_probe, PROBE_ROLE) { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!(&error, json!({})); }
        final_probe = Some(fresh_probe);
        let observed = wait_for_focus_pause(&session);
        let _ = session.quit();
        observed
    } else if args.scenario == Scenario::CleanupFullRelease {
        if !wait_for_startup_ready(&session) { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!("production session did not reach startup_ready before cleanup proof", json!({})); }
        if let Some(code) = finish_preterminal(&args, &session, wait_for_sink_events(&args.sink_events, sink_cursor, &fresh_sink, &expected_down, &[])) { return code; }
        let _ = session.quit();
        false
    } else if args.scenario == Scenario::PauseResume {
        if !wait_for_startup_ready(&session) { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!("production session did not reach startup_ready before pause/resume proof", json!({})); }; let first_pair = expected_physical_keys(plan.profile.as_ref(), &[0]); if let Some(code) = finish_preterminal(&args, &session, wait_for_sink_events(&args.sink_events, sink_cursor, &fresh_sink, &first_pair, &first_pair)) { return code; } first_physical_commit_observed = true; if let Err(error) = session.pause() { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!(&error, json!({})); }; pause_observed = wait_for_focus_pause(&session); if !pause_observed { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!("pause request did not commit after the first physical note pair", json!({})); }; thread::sleep(Duration::from_millis(50)); if let Err(error) = session.resume() { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!(&error, json!({})); }; resume_requested = true; false
    } else if args.scenario == Scenario::TargetHwndChange {
        if !wait_for_startup_ready(&session) { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!("production session did not reach startup_ready before target-change proof", json!({})); } session.set_target_hwnd(0); target_changed = true; false
    } else if args.scenario == Scenario::StopCleanup {
        if !wait_for_startup_ready(&session) { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!("production session did not reach startup_ready before stop proof", json!({})); }; let first_down = expected_physical_keys(plan.profile.as_ref(), &[0]); if let Some(code) = finish_preterminal(&args, &session, wait_for_sink_events(&args.sink_events, sink_cursor, &fresh_sink, &first_down, &[])) { return code; } first_physical_commit_observed = true; if let Err(error) = session.quit() { inconclusive!(&error, json!({})); }; stop_requested = true; false
    } else if args.scenario == Scenario::SkipCleanup {
        if !wait_for_startup_ready(&session) { let _ = session.quit(); let _ = session.join(Duration::from_secs(5)); inconclusive!("production session did not reach startup_ready before skip proof", json!({})); }; let first_down = expected_physical_keys(plan.profile.as_ref(), &[0]); if let Some(code) = finish_preterminal(&args, &session, wait_for_sink_events(&args.sink_events, sink_cursor, &fresh_sink, &first_down, &[])) { return code; } first_physical_commit_observed = true; if let Err(error) = session.skip() { inconclusive!(&error, json!({})); }; skip_requested = true; false
    } else {
        false
    };
    let joined = session.join(Duration::from_secs(if args.scenario == Scenario::ReleaseGapStress { 60 } else { 10 })).unwrap_or(false);
    let snapshot = session.snapshot();
    if !joined { inconclusive!("production session did not join within the bounded timeout", snapshot_json(&snapshot)); }
    let (sink_events, sink_drain_failure) = match drain_event_window(
        &args.sink_events,
        sink_cursor,
        &fresh_sink,
        RECEIVE_ONLY_ROLE,
        DrainMode::ExpectedEvents { allow_unpaired_cleanup_ups: plan.allow_unpaired_cleanup_ups },
        &expected_down,
        &expected_up,
    ) {
        DrainResult::Pass(events) => (events, None),
        DrainResult::Fail(reason, events) => (events, Some(reason)),
        DrainResult::Inconclusive(error) => inconclusive!(&error, snapshot_json(&snapshot)),
    };
    let (probe_events, probe_drain_failure) = match (probe_cursor, args.focus_probe_events.as_deref(), final_probe.as_ref()) {
        (Some(cursor), Some(path), Some(probe)) => match drain_event_window(
            path,
            cursor,
            probe,
            PROBE_ROLE,
            DrainMode::ZeroEventSafety,
            &[],
            &[],
        ) {
            DrainResult::Pass(events) => (events, None),
            DrainResult::Fail(reason, events) => (events, Some(reason)),
            DrainResult::Inconclusive(error) => inconclusive!(&error, snapshot_json(&snapshot)),
        },
        (None, None, None) => (Vec::new(), None),
        _ => inconclusive!("focus probe evidence paths are incomplete", snapshot_json(&snapshot)),
    };
    let mut details = snapshot_json(&snapshot); attach_sink_window_provenance(&mut details, &fresh_sink, sink_cursor, &sink_events, expected_down.len() + expected_up.len(), args.scenario);
    if let Value::Object(object) = &mut details {
        object.insert(
            "focus_probe_event_count".to_string(),
            json!(probe_events.len()),
        );
        object.insert(
            "focus_gate_observed".to_string(),
            json!(focus_gate_observed),
        );
        object.insert("sink_drain_deadline_ms".to_string(), json!(DRAIN_DEADLINE_MS));
        object.insert("sink_drain_quiet_ms".to_string(), json!(DRAIN_QUIET_MS));
        object.insert("probe_zero_event_full_deadline".to_string(), json!(args.scenario.needs_focus_probe()));
        object.insert("authored_packet_targets".to_string(), json!(authored_packet_targets));
        object.insert("expected_down_key_count".to_string(), json!(expected_down.len())); object.insert("expected_up_key_count".to_string(), json!(expected_up.len())); object.insert("control_actions".to_string(), json!({"first_physical_commit_observed": first_physical_commit_observed, "pause_observed": pause_observed, "resume_requested": resume_requested, "target_changed": target_changed, "stop_requested": stop_requested, "skip_requested": skip_requested}));
    }
    let Some(outcome) = snapshot.release_outcome.as_ref() else { inconclusive!("missing cleanup/release evidence", details); };
    if !cleanup_evidence_clean(args.scenario == Scenario::CleanupFullRelease, outcome.attempted_mask, outcome.attempts, outcome.released_successfully, outcome.stuck_mask, outcome.verification_inconclusive, outcome.transport_anomaly) && !target_change_cleanup_exception(&snapshot) {
        return write_report(
            &args,
            Verdict::Fail,
            "cleanup/release evidence is not clean",
            details,
        );
    }
    let expected_target_preflight_failure = args.scenario == Scenario::TargetHwndChange && target_change_preflight_error(&snapshot);
    let target_cleanup_exception = target_change_cleanup_exception(&snapshot);
    if snapshot.active_count != 0 || snapshot.possibly_active_count != 0 || (snapshot.failed_release_count != 0 && !target_cleanup_exception) || snapshot.sendinput_partial_events != 0 || snapshot.sendinput_zero_progress_failures != 0 || (snapshot.terminal_error.is_some() && !expected_target_preflight_failure)
    {
        return write_report(
            &args,
            Verdict::Fail,
            "native session evidence contains a transport or residue anomaly",
            details,
        );
    }
    if let Some(error) = sink_drain_failure {
        return write_report(&args, Verdict::Fail, &error, details);
    }
    if let Some(error) = probe_drain_failure {
        return write_report(&args, Verdict::Fail, &error, details);
    }
    if args.scenario == Scenario::TargetHwndChange && (!target_changed || !target_change_cleanup_exception(&snapshot) || sink_events.iter().any(|event| event.kind == "key_press")) { return write_report(&args, Verdict::Fail, "target HWND transition did not fail closed before gameplay delivery", details); }
    if args.scenario == Scenario::StopCleanup && (!first_physical_commit_observed || snapshot.outcome.as_deref() != Some("quit") || sink_events.is_empty()) { return write_report(&args, Verdict::Fail, "explicit stop did not clean up an active physical key", details); }
    if args.scenario == Scenario::SkipCleanup && (!first_physical_commit_observed || snapshot.outcome.as_deref() != Some("skipped") || sink_events.is_empty()) { return write_report(&args, Verdict::Fail, "explicit skip did not clean up an active physical key", details); }
    if args.scenario == Scenario::PauseResume && (!first_physical_commit_observed || !pause_observed || !resume_requested) { return write_report(&args, Verdict::Fail, "pause/resume control evidence is incomplete", details); }
    if args.scenario == Scenario::RapidRetrigger { let key = expected_physical_keys(plan.profile.as_ref(), &[0])[0]; let expected = [("key_press", key), ("key_release", key), ("key_press", key), ("key_release", key), ("key_press", key), ("key_release", key)]; if let Err(error) = reconcile_event_sequence(&sink_events, &expected) { return write_report(&args, Verdict::Fail, &error, details); } }
    if args.scenario == Scenario::MixedUpDown { let first = expected_physical_keys(plan.profile.as_ref(), &[0])[0]; let second = expected_physical_keys(plan.profile.as_ref(), &[1])[0]; let expected = [("key_press", first), ("key_release", first), ("key_press", second), ("key_release", second)]; if let Err(error) = reconcile_event_sequence(&sink_events, &expected) { return write_report(&args, Verdict::Fail, &error, details); } }
    if args.scenario.needs_focus_probe() {
        let sink_events_clean = reconcile_events(&sink_events, &expected_down, &expected_up, false).is_ok();
        if !focus_evidence_clean(focus_gate_observed, snapshot.final_gate_focus_losses, snapshot.final_gate_target_changes, sink_events_clean, probe_events.is_empty()) {
            return write_report(
                &args,
                Verdict::Fail,
                "authoritative focus-gate evidence was not clean",
                details,
            );
        }
    } else if let Err(error) = reconcile_events(
        &sink_events,
        &expected_down,
        &expected_up,
        plan.allow_unpaired_cleanup_ups,
    ) {
        return write_report(&args, Verdict::Fail, &error, details);
    }
    let (visibility_verdict, visibility_reason) = production_visibility_qualification(
        args.scenario,
        snapshot.production_hold_pair_samples,
        snapshot.production_completion_hold_below_frame_count,
        snapshot.production_release_gap_samples,
        snapshot.production_release_gap_below_policy_count,
    );
    if visibility_verdict != Verdict::Pass { return write_report(&args, visibility_verdict, visibility_reason, details); }
    write_report(
        &args,
        Verdict::Pass,
        "controlled production-path evidence is clean",
        details,
    )
}
#[cfg(not(windows))]
fn run_windows(args: RunArgs) -> i32 {
    write_report(
        &args,
        Verdict::Inconclusive,
        "Windows-only qualification harness",
        json!({"physical_run": false}),
    )
}
pub fn main() {
    match parse_args(env::args().skip(1)) {
        Ok(ParsedCommand::Help) => println!("{}", usage()),
        Ok(ParsedCommand::Run(args)) => std::process::exit(run_windows(*args)),
        Err(error) => {
            eprintln!("{error}\n{}", usage());
            std::process::exit(2);
        }
    }
}
#[cfg(test)]
#[path = "rt_native_acceptance_tests.rs"]
mod tests;
}

fn main() {
    acceptance::main();
}

#![cfg(feature = "real-input-acceptance")]
#[rustfmt::skip]
mod acceptance {
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
use std::collections::HashSet;
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
const ACCEPTANCE_TRANSPORT_MARGIN_US: u64 = 300;
const ACCEPTANCE_MIN_HOLD_US: u64 = ACCEPTANCE_HOLD_FRAMES * ACCEPTANCE_FRAME_US + ACCEPTANCE_DOWN_LATE_GRACE_US + ACCEPTANCE_TRANSPORT_MARGIN_US;
const ACCEPTANCE_MIN_RELEASE_GAP_US: u64 = ACCEPTANCE_FRAME_US + ACCEPTANCE_DOWN_LATE_GRACE_US + ACCEPTANCE_TRANSPORT_MARGIN_US;
const ACCEPTANCE_FOCUS_RESTORE_GRACE_US: u64 = 100_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    CanonicalSingle,
    CanonicalChord,
    CleanupFullRelease,
    FocusLoss,
    W4Noncanonical,
}
impl Scenario {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "canonical-single" => Ok(Self::CanonicalSingle),
            "canonical-chord" => Ok(Self::CanonicalChord),
            "cleanup-full-release" => Ok(Self::CleanupFullRelease),
            "focus-loss" => Ok(Self::FocusLoss),
            "w4-noncanonical" => Ok(Self::W4Noncanonical),
            _ => Err(format!("unsupported scenario: {value}")),
        }
    }
    const fn label(self) -> &'static str {
        match self {
            Self::CanonicalSingle => "canonical-single",
            Self::CanonicalChord => "canonical-chord",
            Self::CleanupFullRelease => "cleanup-full-release",
            Self::FocusLoss => "focus-loss",
            Self::W4Noncanonical => "w4-noncanonical",
        }
    }
    const fn needs_focus_probe(self) -> bool {
        matches!(self, Self::FocusLoss)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct RunArgs {
    run_id: String, sink_ready: PathBuf, sink_events: PathBuf, target_hwnd: isize, scenario: Scenario,
    evidence: PathBuf, focus_probe_ready: Option<PathBuf>, focus_probe_events: Option<PathBuf>, focus_probe_hwnd: Option<isize>,
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
enum Verdict {
    Pass,
    Fail,
    Inconclusive,
}
impl Verdict {
    const fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Inconclusive => "INCONCLUSIVE",
        }
    }
    const fn exit_code(self) -> i32 {
        match self {
            Self::Pass => 0,
            Self::Fail => 1,
            Self::Inconclusive => 2,
        }
    }
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
    "Usage: rt-native-acceptance run --allow-real-input --run-id <id> --sink-ready <path> --sink-events <path> --target-hwnd <decimal|0xhex> --scenario <name> --evidence <path> [--focus-probe-ready <path> --focus-probe-events <path> --focus-probe-hwnd <decimal|0xhex>]"
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
    let mut evidence = None;
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
fn scenario_plan(scenario: Scenario) -> Result<ScenarioPlan, String> {
    let (actions, profile, expected_down_slots, expected_up_slots, allow_unpaired_cleanup_ups) =
        match scenario {
            Scenario::CanonicalSingle | Scenario::W4Noncanonical => (vec![action(0, ActionKind::Down, 50_000, &[0]), action(1, ActionKind::Up, 80_000, &[0])], (scenario == Scenario::W4Noncanonical).then_some(w4_profile_spec()), vec![0], vec![0], false),
            Scenario::CanonicalChord => (vec![action(0, ActionKind::Down, 50_000, &[0, 1]), action(1, ActionKind::Up, 90_000, &[0, 1])], None, vec![0, 1], vec![0, 1], false),
            Scenario::CleanupFullRelease => (vec![action(0, ActionKind::Down, 50_000, &(0..MAX_KEYS).collect::<Vec<_>>()), action(1, ActionKind::Up, 10_000_000, &(0..MAX_KEYS).collect::<Vec<_>>())], None, (0..MAX_KEYS).collect(), (0..MAX_KEYS).collect(), false),
            Scenario::FocusLoss => (vec![action(0, ActionKind::Down, 500_000, &[0]), action(1, ActionKind::Up, 600_000, &[0]), action(2, ActionKind::Down, 1_000_000, &[1]), action(3, ActionKind::Up, 1_100_000, &[1])], None, vec![0], vec![0], false),
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
fn production_options(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    profile: Option<InstrumentKeyProfileSpec>,
) -> NativeSessionOptions {
    NativeSessionOptions {
        schedule,
        backend: BackendConfig::Production,
        profile: DispatchProfile::Production,
        timing: TimingOptions {
            game_fps: 60,
            min_hold_us: ACCEPTANCE_MIN_HOLD_US,
            min_release_gap_us: ACCEPTANCE_MIN_RELEASE_GAP_US,
            down_late_grace_us: ACCEPTANCE_DOWN_LATE_GRACE_US,
            strict_timing: false,
            strict_down_completion_late_us: 2_000,
            strict_up_completion_late_us: 2_000,
            input_path_warn_us: ACCEPTANCE_TRANSPORT_MARGIN_US,
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
    let expected_down = expected_down.iter().copied().collect::<HashSet<_>>();
    let expected_up = expected_up.iter().copied().collect::<HashSet<_>>();
    let mut observed_down = HashSet::new();
    let mut observed_up = HashSet::new();
    for event in events { let physical = physical_event(event); match event.kind.as_str() {
        "key_press" if expected_down.contains(&physical) && observed_down.insert(physical) => {},
        "key_press" => return Err(format!("unexpected or duplicate KeyDown scan=0x{:02X} extended={}", physical.scan_code, physical.extended)),
        "key_release" => { if !expected_up.contains(&physical) || !observed_up.insert(physical) { return Err(format!("unexpected or duplicate KeyUp scan=0x{:02X} extended={}", physical.scan_code, physical.extended)); }
            if !allow_unpaired_cleanup_ups && !observed_down.contains(&physical) { return Err(format!("KeyUp arrived before its matching KeyDown scan=0x{:02X} extended={}", physical.scan_code, physical.extended)); } },
        "sys_key_press" | "sys_key_release" => return Err("SYSKEY event cannot satisfy gameplay evidence".into()),
        other => return Err(format!("unexpected event kind {other:?}")),
    } }
    Ok(observed_down == expected_down && observed_up == expected_up)
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
fn snapshot_json(snapshot: &EngineSnapshot) -> Value {
    let release = snapshot.release_outcome.as_ref().map(|outcome| json!({
        "attempted_mask": outcome.attempted_mask, "transport_anomaly": outcome.transport_anomaly,
        "released_successfully": outcome.released_successfully, "stuck_mask": outcome.stuck_mask,
        "verification_inconclusive": outcome.verification_inconclusive, "attempts": outcome.attempts,
    }));
    json!({"status": snapshot.status, "outcome": snapshot.outcome, "active_count": snapshot.active_count,
        "possibly_active_count": snapshot.possibly_active_count, "failed_release_count": snapshot.failed_release_count,
        "terminal_error": snapshot.terminal_error, "sendinput_partial_events": snapshot.sendinput_partial_events,
        "sendinput_zero_progress_failures": snapshot.sendinput_zero_progress_failures,
        "final_gate_focus_losses": snapshot.final_gate_focus_losses, "final_gate_target_changes": snapshot.final_gate_target_changes,
        "max_sendinput_pre_call_lateness_us": snapshot.max_sendinput_pre_call_lateness_us, "release_outcome": release})
}
fn write_report(args: &RunArgs, verdict: Verdict, reason: &str, details: Value) -> i32 {
    let report = json!({"status": verdict.label(), "scenario": args.scenario.label(), "run_id": args.run_id, "reason": reason, "details": details});
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
    let plan = match scenario_plan(args.scenario) {
        Ok(plan) => plan,
        Err(error) => return write_report(&args, Verdict::Fail, &error, json!({})),
    };
    let expected_down = expected_physical_keys(plan.profile.as_ref(), &plan.expected_down_slots);
    let expected_up = expected_physical_keys(plan.profile.as_ref(), &plan.expected_up_slots);
    let session = match NativeDispatchSession::new(production_options(plan.schedule, plan.profile)) { Ok(session) => session, Err(error) => inconclusive!(&error, json!({})) };
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
    let mut final_probe = targets.probe.clone();
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
    } else {
        false
    };
    let joined = session.join(Duration::from_secs(10)).unwrap_or(false);
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
    let mut details = snapshot_json(&snapshot);
    if let Value::Object(object) = &mut details {
        object.insert("sink_event_count".to_string(), json!(sink_events.len()));
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
    }
    let Some(outcome) = snapshot.release_outcome.as_ref() else {
        inconclusive!("missing cleanup/release evidence", details);
    };
    if !cleanup_evidence_clean(args.scenario == Scenario::CleanupFullRelease, outcome.attempted_mask, outcome.attempts, outcome.released_successfully, outcome.stuck_mask, outcome.verification_inconclusive, outcome.transport_anomaly) {
        return write_report(
            &args,
            Verdict::Fail,
            "cleanup/release evidence is not clean",
            details,
        );
    }
    if snapshot.active_count != 0
        || snapshot.possibly_active_count != 0
        || snapshot.failed_release_count != 0
        || snapshot.sendinput_partial_events != 0
        || snapshot.sendinput_zero_progress_failures != 0
        || snapshot.terminal_error.is_some()
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
mod tests {
    use super::*;
    fn base_arguments(scenario: &str) -> Vec<String> { ["run", "--allow-real-input", "--run-id", "test-run", "--sink-ready", "sink.json", "--sink-events", "sink-events.json", "--target-hwnd", "0x42", "--scenario", scenario, "--evidence", "evidence.jsonl"].into_iter().map(str::to_owned).collect() }
    fn ready(role: &str) -> ReadyRecord { let (sink_kind, title) = if role == RECEIVE_ONLY_ROLE {(SINK_KIND, SINK_TITLE)} else {(PROBE_KIND, PROBE_TITLE)}; ReadyRecord {schema_version: READY_SCHEMA_VERSION, run_id: "test-run".into(), role: role.into(), sink_kind: sink_kind.into(), pid: 7, hwnd: 0x42, title: title.into(), process: SCRIPT_MARKER.into(), input_policy: INPUT_POLICY.into(), event_log_id: "event-stream-test".into(), event_schema_version: EVENT_SCHEMA_VERSION, process_start_time_filetime: 123} }
    fn physical(scan_code: u16) -> PhysicalExpectation { PhysicalExpectation { scan_code, extended: false } }
    fn event_with(sequence: u64, kind: &str, scan_code: u16, extended: bool, virtual_key: i32) -> EventRecord { let message = match kind { "key_press" => 0x0100, "key_release" => 0x0101, "sys_key_press" => 0x0104, "sys_key_release" => 0x0105, _ => 0 }; EventRecord {schema_version: EVENT_SCHEMA_VERSION, run_id: "test-run".into(), role: RECEIVE_ONLY_ROLE.into(), event_log_id: "event-stream-test".into(), sequence, kind: kind.into(), scan_code, extended, virtual_key, message, observed_utc: "2026-01-01T00:00:00Z".into()} }
    fn event(sequence: u64, kind: &str, scan_code: u16) -> EventRecord { event_with(sequence, kind, scan_code, false, 89) }
    #[derive(Debug)] struct FakeReader { samples: Vec<EventWindow>, index: usize }
    impl EventWindowReader for FakeReader { fn read(&mut self) -> Result<EventWindow, String> { let sample = self.samples.get(self.index).or_else(|| self.samples.last()).cloned().ok_or_else(|| "fake reader has no samples".to_string())?; self.index = self.index.saturating_add(1); Ok(sample) } }
    struct FailingReader;
    impl EventWindowReader for FailingReader { fn read(&mut self) -> Result<EventWindow, String> { Err("event log sequence has a duplicate or gap".into()) } }
    #[derive(Debug)] struct FakeClock { now_ms: u64, step_ms: u64 }
    impl DrainClock for FakeClock { fn elapsed_ms(&self) -> u64 { self.now_ms } fn wait_poll(&mut self) { self.now_ms = self.now_ms.saturating_add(self.step_ms); } }
    fn complete_window(events: Vec<EventRecord>) -> EventWindow { EventWindow { events, complete: true } }
    fn partial_window(events: Vec<EventRecord>) -> EventWindow { EventWindow { events, complete: false } }
    #[test] fn schema_v3_rejects_v2_and_requires_bound_header() { let ready = ready(RECEIVE_ONLY_ROLE); let mut old = ready.clone(); old.schema_version = 2; old.event_schema_version = 2; assert!(validate_ready_record(&old, "test-run", 0x42, RECEIVE_ONLY_ROLE).is_err()); assert_eq!(validate_event_stream(&[event(0, "stream_start", 0)], &ready, RECEIVE_ONLY_ROLE), Ok(0)); assert!(validate_event_stream(&[], &ready, RECEIVE_ONLY_ROLE).is_err()); let mut wrong = event(0, "stream_start", 0); wrong.event_log_id = "rebound".into(); assert!(validate_event_stream(&[wrong], &ready, RECEIVE_ONLY_ROLE).is_err()); assert!(validate_event_stream(&[event(0, "stream_start", 0), event(2, "key_press", 0x15)], &ready, RECEIVE_ONLY_ROLE).is_err()); }
    #[test] fn canonical_and_w4_expectations_are_physical() { assert_eq!(expected_physical_keys(None, &[0]), vec![physical(PHYSICAL_INSTRUMENT_SCAN_CODES[0])]); let profile = w4_profile_spec(); assert_eq!(expected_physical_keys(Some(&profile), &[0]), vec![physical(0x02)]); }
    #[test] fn processkey_correct_scan_passes_and_wrong_scan_fails() { let e = [physical(0x15)]; assert!(reconcile_events(&[event_with(1, "key_press", 0x15, false, 0xE5), event_with(2, "key_release", 0x15, false, 0x59)], &e, &e, false).is_ok()); assert!(reconcile_events(&[event_with(1, "key_press", 0x16, false, 0xE5), event_with(2, "key_release", 0x16, false, 0x59)], &e, &e, false).is_err()); }
    #[test] fn physical_reconciliation_rejects_extended_duplicate_direction_and_syskey() { let e = [physical(0x15)]; assert!(reconcile_events(&[event_with(1, "key_press", 0x15, true, 0xE5), event_with(2, "key_release", 0x15, true, 0x59)], &e, &e, false).is_err()); assert!(reconcile_events(&[event(1, "key_press", 0x15), event(2, "key_press", 0x15), event(3, "key_release", 0x15)], &e, &e, false).is_err()); assert!(reconcile_events(&[event_with(1, "sys_key_press", 0x15, false, 0xE5)], &e, &[], false).is_err()); }
    #[test] fn expected_drain_waits_for_late_event_and_quiet() { let e = [physical(0x15)]; let events = vec![event(1, "key_press", 0x15), event(2, "key_release", 0x15)]; let mut r = FakeReader { samples: vec![complete_window(Vec::new()), complete_window(events.clone())], index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert_eq!(drain_event_window_with(&mut r, &mut c, DrainMode::ExpectedEvents { allow_unpaired_cleanup_ups: false }, &e, &e), DrainResult::Pass(events)); }
    #[test] fn expected_drain_missing_fails_and_partial_is_inconclusive() { let e = [physical(0x15)]; let mut r = FakeReader { samples: vec![complete_window(Vec::new())], index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert!(matches!(drain_event_window_with(&mut r, &mut c, DrainMode::ExpectedEvents { allow_unpaired_cleanup_ups: false }, &e, &e), DrainResult::Fail(_, _))); let mut r = FakeReader { samples: vec![partial_window(Vec::new())], index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert!(matches!(drain_event_window_with(&mut r, &mut c, DrainMode::ExpectedEvents { allow_unpaired_cleanup_ups: false }, &e, &e), DrainResult::Inconclusive(_))); }
    #[test] fn preterminal_classification_is_typed_and_fail_closed() { let e = [physical(0x15)]; let events = vec![event(1, "key_press", 0x15), event(2, "key_release", 0x15)]; let mut r = FakeReader { samples: vec![complete_window(Vec::new()), complete_window(events)], index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert_eq!(wait_for_sink_events_with(&mut r, &mut c, &e, &e), PreTerminalResult::Satisfied); let mut r = FakeReader { samples: vec![complete_window(vec![event(1, "key_press", 0x16)])], index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert!(matches!(wait_for_sink_events_with(&mut r, &mut c, &e, &e), PreTerminalResult::TrustedFailure(_))); let mut r = FakeReader { samples: vec![complete_window(vec![event(1, "sys_key_press", 0x15)])], index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert!(matches!(wait_for_sink_events_with(&mut r, &mut c, &e, &e), PreTerminalResult::TrustedFailure(_))); let mut r = FakeReader { samples: vec![complete_window(Vec::new())], index: 0 }; let mut c = FakeClock { now_ms: PRETERMINAL_DEADLINE_MS, step_ms: 10 }; let result = wait_for_sink_events_with(&mut r, &mut c, &e, &e); assert!(matches!(result, PreTerminalResult::IncompleteTimeout { complete: true })); assert_eq!(result.verdict(), Verdict::Fail); let mut r = FakeReader { samples: vec![partial_window(Vec::new())], index: 0 }; let mut c = FakeClock { now_ms: PRETERMINAL_DEADLINE_MS, step_ms: 10 }; let result = wait_for_sink_events_with(&mut r, &mut c, &e, &e); assert!(matches!(result, PreTerminalResult::IncompleteTimeout { complete: false })); assert_eq!(result.verdict(), Verdict::Inconclusive); let mut r = FailingReader; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert!(matches!(wait_for_sink_events_with(&mut r, &mut c, &e, &e), PreTerminalResult::ObservationInconclusive(_))); }
    #[test] fn preterminal_cleanup_verdict_precedence_is_fail_closed() { let clean = NativeCleanupEvidence { terminal_error: false, partial_events: 0, zero_progress_events: 0, active_count: 0, possibly_active_count: 0, failed_release_count: 0, release_failed: false, stuck_mask: 0, verification_inconclusive: false, transport_anomaly: false }; assert_eq!(preterminal_verdict(Verdict::Inconclusive, clean.is_anomalous()), Verdict::Inconclusive); let mut transport = clean; transport.transport_anomaly = true; assert_eq!(preterminal_verdict(Verdict::Inconclusive, transport.is_anomalous()), Verdict::Fail); let mut residue = clean; residue.active_count = 1; residue.possibly_active_count = 1; assert_eq!(preterminal_verdict(Verdict::Inconclusive, residue.is_anomalous()), Verdict::Fail); assert_eq!(preterminal_verdict(Verdict::Fail, clean.is_anomalous()), Verdict::Fail); assert_eq!(preterminal_verdict(Verdict::Fail, transport.is_anomalous()), Verdict::Fail); }
    #[test] fn zero_event_safety_uses_full_deadline_and_catches_delayed_event() { let mut r = FakeReader { samples: vec![complete_window(Vec::new())], index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert_eq!(drain_event_window_with(&mut r, &mut c, DrainMode::ZeroEventSafety, &[], &[]), DrainResult::Pass(Vec::new())); assert_eq!(c.now_ms, DRAIN_DEADLINE_MS); let mut s = vec![complete_window(Vec::new()); 20]; s.push(complete_window(vec![event(1, "key_press", 0x15)])); let mut r = FakeReader { samples: s, index: 0 }; let mut c = FakeClock { now_ms: 0, step_ms: 10 }; assert!(matches!(drain_event_window_with(&mut r, &mut c, DrainMode::ZeroEventSafety, &[], &[]), DrainResult::Fail(_, _))); }
    #[test] fn cleanup_reconciles_fifteen_down_and_up_records() { let e = (0..MAX_KEYS).map(|slot| physical(PHYSICAL_INSTRUMENT_SCAN_CODES[slot])).collect::<Vec<_>>(); let mut a = Vec::new(); for (i, k) in e.iter().enumerate() { a.push(event_with(i as u64 + 1, "key_press", k.scan_code, k.extended, 0)); } for (i, k) in e.iter().enumerate() { a.push(event_with(MAX_KEYS as u64 + i as u64 + 1, "key_release", k.scan_code, k.extended, 0)); } assert!(reconcile_events(&a, &e, &e, false).is_ok()); }
    #[test] fn w4_profile_and_cleanup_verdicts_remain_valid() { let p = MaterializedInstrumentKeyProfile::from_validated(sky_dispatch_win32::input::InstrumentKeyProfile::try_from_spec(w4_profile_spec()).unwrap()); assert_eq!(p.physical_key(0).scan_code, 0x02); let plan = scenario_plan(Scenario::CleanupFullRelease).unwrap(); assert_eq!(plan.schedule.packets[0].down_mask, FULL_INSTRUMENT_MASK); assert!(cleanup_evidence_clean(true, FULL_INSTRUMENT_MASK, 1, true, 0, false, false)); }
    #[test] fn authorization_and_timing_contracts_remain_bounded() { let mut a = base_arguments("focus-loss"); a.extend(["--focus-probe-ready", "p", "--focus-probe-events", "e", "--focus-probe-hwnd", "0x43"].into_iter().map(str::to_owned)); assert!(parse_args(a).is_ok()); assert_eq!(ACCEPTANCE_MIN_HOLD_US, 17_467); assert_eq!(ACCEPTANCE_MIN_RELEASE_GAP_US, 17_467); assert!(focus_evidence_clean(true, 1, 0, true, true)); }
}
}
fn main() {
    acceptance::main();
}

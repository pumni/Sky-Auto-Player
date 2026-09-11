#![cfg(feature = "real-input-acceptance")]
#[rustfmt::skip]
mod acceptance {
use serde::Deserialize;
use serde_json::{Value, json};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, MAX_KEYS};
use sky_dispatch_win32::focus::{
    WindowIdentity, focus_window_and_verify, inspect_window_identity, virtual_key_for_scan_code,
};
#[cfg(test)]
use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;
use sky_dispatch_win32::input::{
    InstrumentKeyProfileSpec, PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalKey,
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
const READY_SCHEMA_VERSION: u32 = 2;
const RECEIVE_ONLY_ROLE: &str = "ReceiveOnly";
const PROBE_ROLE: &str = "InertFocusProbe";
const SINK_KIND: &str = "SkyAutoPlayer.NativeAcceptanceSink";
const PROBE_KIND: &str = "SkyAutoPlayer.NativeAcceptanceFocusProbe";
const SINK_TITLE: &str = "Sky Auto Player — Native Acceptance Sink";
const PROBE_TITLE: &str = "Sky Auto Player — Native Acceptance Focus Probe";
const SCRIPT_MARKER: &str = "native_acceptance_sink.ps1";
const INPUT_POLICY: &str = concat!("receives_only; benchmark must use ", "Send", "Input");
const REQUIRED_IMAGE_BASENAME: &str = "pwsh.exe";
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
    const fn cleanup_is_full(self) -> bool {
        matches!(self, Self::CleanupFullRelease | Self::FocusLoss)
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
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
struct ReadyRecord {
    schema_version: u32, run_id: String, role: String, sink_kind: String, pid: u32, hwnd: i64,
    title: String, process: String, input_policy: String, process_start_time_filetime: u64,
}
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
struct EventRecord {
    run_id: String, sequence: u64, kind: String, key_code: i32, observed_utc: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LogCursor {
    offset: u64,
    sequence: u64,
}
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
    if record.schema_version != READY_SCHEMA_VERSION { return Err("ready-file schema_version must be 2".into()); }
    if record.run_id != run_id { return Err("ready-file run_id does not match --run-id".into()); }
    if record.role != expected_role
        || record.sink_kind != expected_kind
        || record.title != expected_title
        || record.process != SCRIPT_MARKER
        || record.input_policy != INPUT_POLICY
    {
        return Err("ready-file protocol markers do not match the expected project-owned role".into());
    }
    let ready_hwnd = isize::try_from(record.hwnd).map_err(|_| "ready-file HWND is outside the isize range".to_string())?;
    if ready_hwnd <= 0 || ready_hwnd != target_hwnd { return Err("ready-file HWND does not match the explicit target HWND".into()); }
    if record.pid == 0 || record.process_start_time_filetime == 0 { return Err("ready-file process identity is incomplete".into()); }
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
            Scenario::CanonicalSingle | Scenario::W4Noncanonical => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 80_000, &[0]),
                ],
                (scenario == Scenario::W4Noncanonical).then_some(w4_profile_spec()),
                vec![0],
                vec![0],
                false,
            ),
            Scenario::CanonicalChord => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0, 1]),
                    action(1, ActionKind::Up, 90_000, &[0, 1]),
                ],
                None,
                vec![0, 1],
                vec![0, 1],
                false,
            ),
            Scenario::CleanupFullRelease => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0, 1, 2]),
                    action(1, ActionKind::Up, 500_000, &[0, 1, 2]),
                ],
                None,
                vec![0, 1, 2],
                (0..MAX_KEYS).collect(),
                true,
            ),
            Scenario::FocusLoss => (
                vec![
                    action(0, ActionKind::Down, 250_000, &[0]),
                    action(1, ActionKind::Up, 350_000, &[0]),
                ],
                None,
                Vec::new(),
                Vec::new(),
                false,
            ),
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
            min_hold_us: 10_000,
            min_release_gap_us: 16_667,
            down_late_grace_us: 500,
            strict_timing: false,
            strict_down_completion_late_us: 2_000,
            strict_up_completion_late_us: 2_000,
            input_path_warn_us: 300,
        },
        focus: FocusOptions {
            require_focus: true,
            focus_restore_grace_us: 100_000,
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
fn profile_scan_code(profile: Option<&InstrumentKeyProfileSpec>, slot: usize) -> u16 {
    profile
        .map(|profile| profile.keys[slot].scan_code)
        .unwrap_or(PHYSICAL_INSTRUMENT_SCAN_CODES[slot])
}
fn expected_key_codes(
    target_hwnd: isize,
    profile: Option<&InstrumentKeyProfileSpec>,
    slots: &[usize],
) -> Result<Vec<i32>, String> {
    slots
        .iter()
        .map(|slot| virtual_key_for_scan_code(target_hwnd, profile_scan_code(profile, *slot)))
        .collect()
}
fn read_log_cursor(path: &Path, run_id: &str) -> Result<LogCursor, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read event log: {error}"))?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err("event log does not end at a complete JSONL record".to_string());
    }
    let text =
        String::from_utf8(bytes.clone()).map_err(|_| "event log is not valid UTF-8".to_string())?;
    let mut sequence = 0;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let event: EventRecord = serde_json::from_str(line)
            .map_err(|error| format!("event log contains non-schema-v2 JSON: {error}"))?;
        if event.run_id == run_id {
            sequence = sequence.max(event.sequence);
        }
    }
    Ok(LogCursor {
        offset: bytes.len() as u64,
        sequence,
    })
}
fn read_log_window(
    path: &Path,
    cursor: LogCursor,
    run_id: &str,
) -> Result<Vec<EventRecord>, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("failed to read event log window: {error}"))?;
    let bytes = bytes
        .get(cursor.offset as usize..)
        .ok_or_else(|| "event log was truncated after authorization".to_string())?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err("event log window ends with a truncated record".to_string());
    }
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| "event log window is not UTF-8".to_string())?;
    let mut previous_sequence = cursor.sequence;
    let mut events = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let event: EventRecord = serde_json::from_str(line)
            .map_err(|error| format!("event log window contains malformed JSON: {error}"))?;
        if event.run_id != run_id {
            return Err("event log window contains a different run_id".to_string());
        }
        if event.sequence != previous_sequence.saturating_add(1) {
            return Err("event log sequence has a duplicate or gap".to_string());
        }
        previous_sequence = event.sequence;
        events.push(event);
    }
    Ok(events)
}
fn reconcile_events(
    events: &[EventRecord],
    expected_down: &[i32],
    expected_up: &[i32],
    allow_unpaired_cleanup_ups: bool,
) -> Result<(), String> {
    let expected_down = expected_down.iter().copied().collect::<HashSet<_>>();
    let expected_up = expected_up.iter().copied().collect::<HashSet<_>>();
    let mut observed_down = HashSet::new();
    let mut observed_up = HashSet::new();
    for event in events {
        match event.kind.as_str() {
            "key_press"
                if expected_down.contains(&event.key_code)
                    && observed_down.insert(event.key_code) => {}
            "key_press" => {
                return Err(format!(
                    "unexpected or duplicate KeyDown {}",
                    event.key_code
                ));
            }
            "key_release" => {
                if !expected_up.contains(&event.key_code) || !observed_up.insert(event.key_code) {
                    return Err(format!("unexpected or duplicate KeyUp {}", event.key_code));
                }
                if !allow_unpaired_cleanup_ups && !observed_down.contains(&event.key_code) {
                    return Err(format!(
                        "KeyUp {} arrived before its matching KeyDown",
                        event.key_code
                    ));
                }
            }
            other => return Err(format!("unexpected event kind {other:?}")),
        }
    }
    if observed_down != expected_down {
        return Err("event log is missing an expected KeyDown".into());
    }
    if observed_up != expected_up {
        return Err("event log is missing an expected KeyUp".into());
    }
    Ok(())
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
fn wait_for_focus_rejection(session: &NativeDispatchSession) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if session.snapshot().final_gate_focus_losses > 0 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
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
    let sink_cursor = match read_log_cursor(&args.sink_events, &args.run_id) {
        Ok(cursor) => cursor,
        Err(error) => inconclusive!(&error, json!({})),
    };
    let probe_cursor = match targets.probe.as_ref() {
        Some(_) => match read_log_cursor(
            args.focus_probe_events
                .as_deref()
                .expect("focus probe event path"),
            &args.run_id,
        ) {
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
    let expected_down = match expected_key_codes(sink_hwnd, plan.profile.as_ref(), &plan.expected_down_slots) { Ok(keys) => keys, Err(error) => inconclusive!(&error, json!({})) };
    let expected_up = match expected_key_codes(sink_hwnd, plan.profile.as_ref(), &plan.expected_up_slots) { Ok(keys) => keys, Err(error) => inconclusive!(&error, json!({})) };
    let session = match NativeDispatchSession::new(production_options(plan.schedule, plan.profile)) { Ok(session) => session, Err(error) => inconclusive!(&error, json!({})) };
    session.set_target_hwnd(sink_hwnd);
    session.set_focus_hint(true);
    if let Err(error) = session.arm(0) {
        inconclusive!(&error, json!({}));
    }
    let focus_gate_observed = if args.scenario.needs_focus_probe() {
        let probe_hwnd = targets.probe.as_ref().expect("focus scenario probe").hwnd as isize;
        if !focus_window_and_verify(probe_hwnd, Duration::from_millis(250)) {
            let _ = session.quit();
            let _ = session.join(Duration::from_secs(5));
            inconclusive!("focus probe could not become the exact foreground HWND", json!({}));
        }
        let observed = wait_for_focus_rejection(&session);
        let _ = session.quit();
        observed
    } else if args.scenario.cleanup_is_full() {
        thread::sleep(Duration::from_millis(150));
        let _ = session.quit();
        false
    } else {
        false
    };
    let joined = session.join(Duration::from_secs(10)).unwrap_or(false);
    let snapshot = session.snapshot();
    if !joined { inconclusive!("production session did not join within the bounded timeout", snapshot_json(&snapshot)); }
    let sink_events = match read_log_window(&args.sink_events, sink_cursor, &args.run_id) {
        Ok(events) => events,
        Err(error) => inconclusive!(&error, snapshot_json(&snapshot)),
    };
    let probe_events = match (probe_cursor, args.focus_probe_events.as_deref()) {
        (Some(cursor), Some(path)) => match read_log_window(path, cursor, &args.run_id) {
            Ok(events) => events,
            Err(error) => inconclusive!(&error, snapshot_json(&snapshot)),
        },
        (None, None) => Vec::new(),
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
    }
    let Some(outcome) = snapshot.release_outcome.as_ref() else {
        inconclusive!("missing cleanup/release evidence", details);
    };
    if !outcome.released_successfully
        || outcome.stuck_mask != 0
        || outcome.verification_inconclusive
        || outcome.transport_anomaly
    {
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
    if args.scenario.needs_focus_probe() {
        if !focus_gate_observed
            || snapshot.final_gate_focus_losses == 0
            || snapshot.final_gate_target_changes != 0
            || !sink_events.is_empty()
            || !probe_events.is_empty()
        {
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
    fn base_arguments(scenario: &str) -> Vec<String> {
        ["run", "--allow-real-input", "--run-id", "test-run", "--sink-ready", "sink.json", "--sink-events", "sink-events.json", "--target-hwnd", "0x42", "--scenario", scenario, "--evidence", "evidence.jsonl"].into_iter().map(str::to_owned).collect()
    }
    fn ready(role: &str) -> ReadyRecord {
        let (sink_kind, title) = if role == RECEIVE_ONLY_ROLE {(SINK_KIND, SINK_TITLE)} else {(PROBE_KIND, PROBE_TITLE)};
        ReadyRecord {schema_version: READY_SCHEMA_VERSION, run_id: "test-run".into(), role: role.into(), sink_kind: sink_kind.into(), pid: 7, hwnd: 0x42, title: title.into(), process: SCRIPT_MARKER.into(), input_policy: INPUT_POLICY.into(), process_start_time_filetime: 123}
    }
    fn event(sequence: u64, kind: &str, key_code: i32) -> EventRecord {
        EventRecord {run_id: "test-run".into(), sequence, kind: kind.into(), key_code, observed_utc: "2026-01-01T00:00:00Z".into()}
    }
    #[test]
    fn cli_requires_authorization_target_and_complete_focus_evidence() {
        let mut args = base_arguments("canonical-single");
        args.retain(|value| value != "--allow-real-input");
        assert!(parse_args(args).is_err());
        let mut args = base_arguments("canonical-single");
        let index = args
            .iter()
            .position(|value| value == "--target-hwnd")
            .unwrap();
        args.remove(index + 1);
        assert!(parse_args(args).is_err());
        let mut args = base_arguments("focus-loss");
        args.extend(["--focus-probe-ready".into(), "probe.json".into()]);
        assert!(parse_args(args.clone()).is_err());
        args.extend(["--focus-probe-events", "probe-events.json", "--focus-probe-hwnd", "0x43"].into_iter().map(str::to_owned));
        assert!(matches!(parse_args(args), Ok(ParsedCommand::Run(_))));
    }
    #[test]
    fn hwnd_parser_accepts_decimal_and_hex_but_rejects_zero() {
        assert_eq!(parse_hwnd("66"), Ok(66));
        assert_eq!(parse_hwnd("0x42"), Ok(66));
        assert!(parse_hwnd("0").is_err());
        assert!(parse_hwnd("not-a-hwnd").is_err());
    }
    #[test]
    fn protocol_and_live_identity_require_exact_markers_and_numeric_freshness() {
        let record = ready(RECEIVE_ONLY_ROLE);
        assert!(validate_ready_record(&record, "test-run", 0x42, RECEIVE_ONLY_ROLE).is_ok());
        let mut changed = record.clone();
        changed.process = "pwsh.exe".into();
        assert!(validate_ready_record(&changed, "test-run", 0x42, RECEIVE_ONLY_ROLE).is_err());
        let mut changed = record;
        changed.process_start_time_filetime = 0;
        assert!(validate_ready_record(&changed, "test-run", 0x42, RECEIVE_ONLY_ROLE).is_err());
        let record = ready(RECEIVE_ONLY_ROLE);
        let live = WindowIdentity {hwnd: 0x42, owner_pid: 7, title: SINK_TITLE.into(), process_start_time_filetime: 123, process_image_basename: REQUIRED_IMAGE_BASENAME.into()};
        assert!(validate_live_identity(&record, &live).is_ok());
        let mut changed = live.clone();
        changed.process_image_basename = "powershell.exe".into();
        assert!(validate_live_identity(&record, &changed).is_err());
        let mut changed = live;
        changed.process_start_time_filetime += 1;
        assert!(validate_live_identity(&record, &changed).is_err());
    }
    #[test]
    fn scenario_plan_keeps_logical_slots_and_w4_profile_separate() {
        let plan = scenario_plan(Scenario::W4Noncanonical).expect("scenario plan");
        assert_eq!(plan.expected_down_slots, vec![0]);
        assert_eq!(plan.expected_up_slots, vec![0]);
        assert_eq!(plan.profile.unwrap().keys[0].scan_code, 0x02);
        assert_eq!(plan.schedule.packets[0].down_mask, 1);
    }
    #[test]
    fn reconciliation_rejects_invalid_order_and_allows_declared_cleanup_ups() {
        assert!(reconcile_events(&[event(1, "key_press", 89), event(2, "key_release", 89)], &[89], &[89], false).is_ok());
        assert!(reconcile_events(&[event(1, "key_press", 89)], &[89], &[89], false).is_err());
        assert!(reconcile_events(&[event(1, "key_release", 89), event(2, "key_press", 89)], &[89], &[89], false).is_err());
        assert!(reconcile_events(&[event(1, "key_press", 89), event(2, "key_press", 89)], &[89], &[89], false).is_err());
        assert!(reconcile_events(&[event(1, "key_press", 89), event(2, "key_release", 89), event(3, "key_release", 85)], &[89], &[89, 85], true).is_ok());
        assert!(reconcile_events(&[event(1, "key_release", 90)], &[], &[89], true,).is_err());
    }
    #[test]
    fn materialized_w4_profile_is_valid_without_test_transport() {
        let profile = MaterializedInstrumentKeyProfile::from_validated(
            sky_dispatch_win32::input::InstrumentKeyProfile::try_from_spec(w4_profile_spec())
                .expect("W4 profile"),
        );
        assert_eq!(profile.physical_key(0).scan_code, 0x02);
        assert!(!profile.physical_key(0).extended);
    }
}
}
fn main() {
    acceptance::main();
}

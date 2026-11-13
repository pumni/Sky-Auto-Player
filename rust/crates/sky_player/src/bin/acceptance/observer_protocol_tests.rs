use super::{
    EVENT_SCHEMA_VERSION, EventRecord, EventWindow, LogCursor, RECEIVE_ONLY_ROLE, ReadyRecord,
    read_log_window_status, validate_event_stream, validate_ready_record,
};
#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;

const OBSERVER_MODE: &str = "SchedulerQueuedV3Compat3";
const OBSERVER_VERSION: &str = "scheduler-receiveonly-queue-v3-compat3";

#[cfg(windows)]
struct FixtureRoot {
    path: PathBuf,
    remove_on_drop: bool,
}

#[cfg(windows)]
impl Drop for FixtureRoot {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(windows)]
fn emitted_fixture_root() -> FixtureRoot {
    if let Some(path) = std::env::var_os("SKY_NATIVE_V3_COMPAT3_FIXTURE_ROOT") {
        return FixtureRoot { path: PathBuf::from(path), remove_on_drop: false };
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "native-v3-compat3-rust-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create external serializer fixture directory");
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root
        .ancestors()
        .nth(3)
        .expect("sky_player crate is nested under the workspace root");
    let sink_script = repository_root.join("scripts/native_acceptance_sink.ps1");
    let output = Command::new("pwsh.exe")
        .args(["-NoProfile", "-File"])
        .arg(&sink_script)
        .args(["-SelfTest", "-SelfTestFixtureRoot"])
        .arg(&path)
        .output()
        .expect("run the PowerShell serializer fixture without sending input");
    assert!(
        output.status.success(),
        "PowerShell serializer fixture failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    FixtureRoot { path, remove_on_drop: true }
}

fn ready_v3() -> ReadyRecord {
    ReadyRecord {
        schema_version: 3,
        run_id: "compat3-test".into(),
        role: RECEIVE_ONLY_ROLE.into(),
        sink_kind: "SkyAutoPlayer.NativeAcceptanceSink".into(),
        pid: 7,
        hwnd: 66,
        title: "Sky Auto Player — Native Acceptance Sink".into(),
        process: "native_acceptance_sink.ps1".into(),
        input_policy: "receives_only; benchmark must use SendInput".into(),
        event_log_id: "compat3-event-stream".into(),
        event_schema_version: 3,
        process_start_time_filetime: 123,
    }
}

fn event_v3(sequence: u64, kind: &str, scan_code: u16, virtual_key: i32, message: u32) -> EventRecord {
    EventRecord {
        schema_version: EVENT_SCHEMA_VERSION,
        run_id: "compat3-test".into(),
        role: RECEIVE_ONLY_ROLE.into(),
        event_log_id: "compat3-event-stream".into(),
        sequence,
        kind: kind.into(),
        scan_code,
        extended: false,
        virtual_key,
        message,
        observed_utc: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
fn native_schema3_reader_accepts_v3_compat3_json_extensions() {
    let ready: ReadyRecord = serde_json::from_value(serde_json::json!({
        "schema_version": 3,
        "event_schema_version": 3,
        "run_id": "compat3-test",
        "role": "ReceiveOnly",
        "sink_kind": "SkyAutoPlayer.NativeAcceptanceSink",
        "pid": 7,
        "hwnd": 66,
        "title": "Sky Auto Player — Native Acceptance Sink",
        "process": "native_acceptance_sink.ps1",
        "input_policy": "receives_only; benchmark must use SendInput",
        "event_log_id": "compat3-event-stream",
        "process_start_time_filetime": 123,
        "observer_mode": OBSERVER_MODE,
        "observer_version": OBSERVER_VERSION,
        "observer_queue_capacity": 32768,
        "observer_qpc_frequency_hz": 10000000
    }))
    .expect("schema-3 ready required fields deserialize with Compat3 extensions");
    assert!(validate_ready_record(&ready, "compat3-test", 66, RECEIVE_ONLY_ROLE).is_ok());

    let header: EventRecord = serde_json::from_value(serde_json::json!({
        "schema_version": 3,
        "run_id": "compat3-test",
        "role": "ReceiveOnly",
        "event_log_id": "compat3-event-stream",
        "sequence": 0,
        "kind": "stream_start",
        "scan_code": 0,
        "extended": false,
        "virtual_key": 0,
        "message": 0,
        "observed_utc": "2026-01-01T00:00:00Z",
        "observer_mode": OBSERVER_MODE,
        "observer_version": OBSERVER_VERSION,
        "observed_qpc_ticks": 100,
        "target_hwnd": 66,
        "target_pid": 7,
        "foreground_hwnd": 55,
        "foreground_pid": 8
    }))
    .expect("schema-3 header extensions are ignored by the pinned reader");
    let keyboard: EventRecord = serde_json::from_value(serde_json::json!({
        "schema_version": 3,
        "run_id": "compat3-test",
        "role": "ReceiveOnly",
        "event_log_id": "compat3-event-stream",
        "sequence": 1,
        "kind": "key_press",
        "scan_code": 0x15,
        "extended": true,
        "virtual_key": 0x41,
        "message": 0x0100,
        "observed_utc": "2026-01-01T00:00:00.0000010Z",
        "observer_mode": OBSERVER_MODE,
        "observer_version": OBSERVER_VERSION,
        "observed_qpc_ticks": 101,
        "target_hwnd": 66,
        "target_pid": 7,
        "foreground_hwnd": 66,
        "foreground_pid": 7
    }))
    .expect("schema-3 keyboard extensions are ignored by the pinned reader");
    assert_eq!(keyboard.virtual_key, 0x41);
    assert_eq!(keyboard.message, 0x0100);
    assert_eq!(
        validate_event_stream(&[header, keyboard], &ready, RECEIVE_ONLY_ROLE),
        Ok(1)
    );
}

#[test]
fn pinned_native_schema3_reader_rejects_v2_schema4_ready_and_keyboard() {
    let ready = ready_v3();
    let mut schema4_ready = ready.clone();
    schema4_ready.event_schema_version = 4;
    assert_eq!(
        validate_ready_record(&schema4_ready, "compat3-test", 66, RECEIVE_ONLY_ROLE)
            .expect_err("schema-4 ready must fail closed"),
        "ready-file schema/event schema version must be 3"
    );

    let header = event_v3(0, "stream_start", 0, 0, 0);
    let mut schema4_keyboard = event_v3(1, "key_press", 0x15, 0x41, 0x0100);
    schema4_keyboard.schema_version = 4;
    assert_eq!(
        validate_event_stream(&[header, schema4_keyboard], &ready, RECEIVE_ONLY_ROLE)
            .expect_err("schema-4 keyboard event must fail closed"),
        "event log is not bound to the ready-file process stream"
    );
}

#[test]
#[cfg(windows)]
fn native_reader_accepts_powershell_v3_compat3_fixture_and_rolling_window() {
    let root = emitted_fixture_root();
    let ready_bytes = fs::read(root.path.join("ready.json")).expect("read emitted ready fixture");
    let ready: ReadyRecord = serde_json::from_slice(&ready_bytes).expect("decode ready fixture");
    validate_ready_record(&ready, "v3-compat3-contract", 42, RECEIVE_ONLY_ROLE)
        .expect("native reader accepts the emitted schema-3 ready record");

    let event_path = root.path.join("events.jsonl");
    let event_bytes = fs::read(&event_path).expect("read emitted event JSONL fixture");
    let text = std::str::from_utf8(&event_bytes).expect("event fixture is UTF-8");
    assert!(text.ends_with('\n'), "fixture must end at a complete JSONL record");
    let records = text
        .lines()
        .map(|line| serde_json::from_str::<EventRecord>(line).expect("decode emitted record"))
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 5, "header plus four captured message fixtures");
    validate_event_stream(&records, &ready, RECEIVE_ONLY_ROLE)
        .expect("native event-stream validator accepts serializer output");
    assert_eq!(records[1].message, 0x0100);
    assert_eq!(records[2].message, 0x0101);
    assert_eq!(records[3].message, 0x0104);
    assert_eq!(records[4].message, 0x0105);
    assert_eq!(records[1].virtual_key, 0x41);
    assert_eq!(records[3].virtual_key, 0x12);
    assert!(records[1..].iter().all(|record| record.scan_code != 0));

    let header_bytes = event_bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header has a JSONL terminator")
        + 1;
    let window = read_log_window_status(
        &event_path,
        LogCursor {
            offset: header_bytes as u64,
            sequence: 0,
        },
        &ready,
        RECEIVE_ONLY_ROLE,
    )
    .expect("rolling event-window reader accepts emitted schema-3 extensions");
    assert_eq!(window, EventWindow { events: records[1..].to_vec(), complete: true });
}

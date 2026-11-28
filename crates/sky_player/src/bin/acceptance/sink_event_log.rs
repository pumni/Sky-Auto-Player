use super::{
    EVENT_SCHEMA_VERSION, EventRecord, EventWindow, LogCursor, ReadyRecord, parse_event_log,
    validate_event_stream,
};
use std::fs;
use std::path::Path;

pub(super) fn read_log_window_status(
    path: &Path,
    cursor: LogCursor,
    ready: &ReadyRecord,
    role: &str,
) -> Result<EventWindow, String> {
    let all = fs::read(path).map_err(|error| format!("failed to read event log window: {error}"))?;
    let all_complete_len = if all.ends_with(b"\n") {
        all.len()
    } else {
        all.iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1)
    };
    let prefix = parse_event_log(&all[..all_complete_len], "event log window prefix")?;
    let prefix_sequence = validate_event_stream(&prefix, ready, role)?;
    if all.len() < cursor.offset as usize || prefix_sequence < cursor.sequence {
        return Err("event log was truncated or rebound after authorization".into());
    }
    let bytes = all
        .get(cursor.offset as usize..)
        .ok_or_else(|| "event log was truncated after authorization".to_string())?;
    let complete = bytes.is_empty() || bytes.ends_with(b"\n");
    let complete_len = if complete {
        bytes.len()
    } else {
        bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1)
    };
    let text = String::from_utf8(bytes[..complete_len].to_vec())
        .map_err(|_| "event log window is not UTF-8".to_string())?;
    let mut previous_sequence = cursor.sequence;
    let mut events = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let event: EventRecord = serde_json::from_str(line)
            .map_err(|error| format!("event log window contains malformed JSON: {error}"))?;
        if event.run_id != ready.run_id
            || event.role != role
            || event.event_log_id != ready.event_log_id
            || event.schema_version != EVENT_SCHEMA_VERSION
        {
            return Err("event log window is not bound to the authorized process stream".into());
        }
        if event.sequence != previous_sequence.saturating_add(1) {
            return Err("event log sequence has a duplicate or gap".to_string());
        }
        previous_sequence = event.sequence;
        if event.kind != "stream_start" {
            events.push(event);
        }
    }
    Ok(EventWindow { events, complete })
}

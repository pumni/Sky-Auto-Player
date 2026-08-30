use crate::ui_events::{
    CalibrationFinishedPayload, CalibrationProgressPayload, CatalogChangedPayload,
    CoreFatalPayload, CoreReadyPayload, DiagnosticsSnapshotDto, PlaybackFailedPayload,
    PlaybackFinishedPayload, PlaybackSnapshotPayload, PlaybackStateChangedPayload, UiEvent,
    UpdateAvailablePayload, UpdateHandoffReadyPayload, UpdateResultPayload,
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::fmt;
use std::io::{self, Read};

pub const DESKTOP_PROTOCOL_VERSION: u64 = 1;
pub const MAX_INBOUND_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_OUTBOUND_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_REQUEST_ID: u64 = 2_u64.pow(53) - 1;
const READ_CHUNK_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreErrorPayload {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreResponse {
    pub id: u64,
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<CoreErrorPayload>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    Ready(CoreReadyPayload),
    Fatal(CoreFatalPayload),
    CatalogChanged(CatalogChangedPayload),
    PlaybackStateChanged(PlaybackStateChangedPayload),
    PlaybackSnapshot(PlaybackSnapshotPayload),
    PlaybackFinished(PlaybackFinishedPayload),
    PlaybackFailed(PlaybackFailedPayload),
    DiagnosticsSnapshot(DiagnosticsSnapshotDto),
    CalibrationProgress(CalibrationProgressPayload),
    CalibrationFinished(CalibrationFinishedPayload),
    UpdateAvailable(UpdateAvailablePayload),
    UpdateResult(UpdateResultPayload),
    UpdateHandoffReady(UpdateHandoffReadyPayload),
}

impl CoreEvent {
    pub fn into_ui_event(self) -> UiEvent {
        match self {
            Self::Ready(payload) => UiEvent::CoreReady {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::Fatal(payload) => UiEvent::CoreFatal {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::CatalogChanged(payload) => UiEvent::CatalogChanged {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::PlaybackStateChanged(payload) => UiEvent::PlaybackStateChanged {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::PlaybackSnapshot(payload) => UiEvent::PlaybackSnapshot {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::PlaybackFinished(payload) => UiEvent::PlaybackFinished {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::PlaybackFailed(payload) => UiEvent::PlaybackFailed {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::DiagnosticsSnapshot(payload) => UiEvent::DiagnosticsSnapshot {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::CalibrationProgress(payload) => UiEvent::CalibrationProgress {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::CalibrationFinished(payload) => UiEvent::CalibrationFinished {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::UpdateAvailable(payload) => UiEvent::UpdateAvailable {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::UpdateResult(payload) => UiEvent::UpdateResult {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
            Self::UpdateHandoffReady(payload) => UiEvent::UpdateHandoffReady {
                v: DESKTOP_PROTOCOL_VERSION,
                payload,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreMessage {
    Response(CoreResponse),
    Event(CoreEvent),
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("I/O error while reading Core frame: {0}")]
    Io(#[from] io::Error),
    #[error("Core frame exceeds {0} bytes")]
    FrameTooLarge(usize),
    #[error("invalid Core JSON: {0}")]
    Json(String),
    #[error("invalid Core protocol message: {0}")]
    Invalid(String),
}

struct StrictValue;

impl<'de> DeserializeSeed<'de> for StrictValue {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique object keys")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
            }
            let value = access.next_value_seed(StrictValue)?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(StrictValue)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(Value::from(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }
}

pub fn encode_request(id: u64, method: &str, params: Value) -> Result<Vec<u8>, ProtocolError> {
    if id > MAX_REQUEST_ID {
        return Err(ProtocolError::Invalid(
            "request id is outside the JSON-safe range".into(),
        ));
    }
    if method.is_empty() || method.len() > 128 {
        return Err(ProtocolError::Invalid(
            "method is outside the 128-byte limit".into(),
        ));
    }
    let mut object = Map::new();
    object.insert("v".into(), Value::from(DESKTOP_PROTOCOL_VERSION));
    object.insert("id".into(), Value::from(id));
    object.insert("type".into(), Value::from("request"));
    object.insert("method".into(), Value::from(method));
    object.insert("params".into(), params);
    let mut encoded = serde_json::to_vec(&Value::Object(object))
        .map_err(|error| ProtocolError::Json(error.to_string()))?;
    encoded.push(b'\n');
    if encoded.len() > MAX_INBOUND_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge(MAX_INBOUND_FRAME_BYTES));
    }
    Ok(encoded)
}

fn parse_object(frame: &[u8]) -> Result<Map<String, Value>, ProtocolError> {
    let mut deserializer = serde_json::Deserializer::from_slice(frame);
    let value = StrictValue
        .deserialize(&mut deserializer)
        .map_err(|error| ProtocolError::Json(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| ProtocolError::Json(error.to_string()))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| ProtocolError::Invalid("protocol message must be a JSON object".into()))
}

fn ensure_fields(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), ProtocolError> {
    if let Some(unexpected) = object
        .keys()
        .find(|key| !allowed.iter().any(|allowed| allowed == key))
    {
        return Err(ProtocolError::Invalid(format!(
            "unexpected protocol field: {unexpected}"
        )));
    }
    Ok(())
}

fn required_string(object: &Map<String, Value>, name: &str) -> Result<String, ProtocolError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProtocolError::Invalid(format!("{name} must be a string")))
}

fn required_u64(object: &Map<String, Value>, name: &str) -> Result<u64, ProtocolError> {
    object
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| ProtocolError::Invalid(format!("{name} must be an unsigned integer")))
}

fn parse_event(name: &str, payload: Value) -> Result<CoreEvent, ProtocolError> {
    fn decode<T>(name: &str, payload: Value) -> Result<T, ProtocolError>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_value(payload)
            .map_err(|error| ProtocolError::Invalid(format!("invalid {name} payload: {error}")))
    }

    match name {
        "core.ready" => {
            let value: CoreReadyPayload = decode(name, payload)?;
            if value.protocol_version != DESKTOP_PROTOCOL_VERSION {
                return Err(ProtocolError::Invalid(
                    "core.ready protocol_version does not match the desktop protocol".into(),
                ));
            }
            UiEvent::validate_ready(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::Ready(value))
        }
        "core.fatal" => {
            let value: CoreFatalPayload = decode(name, payload)?;
            UiEvent::validate_fatal(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::Fatal(value))
        }
        "catalog.changed" => {
            let value: CatalogChangedPayload = decode(name, payload)?;
            UiEvent::validate_catalog_changed(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::CatalogChanged(value))
        }
        "playback.state_changed" => {
            let value: PlaybackStateChangedPayload = decode(name, payload)?;
            UiEvent::validate_playback_state_changed(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::PlaybackStateChanged(value))
        }
        "playback.snapshot" => {
            let value: PlaybackSnapshotPayload = decode(name, payload)?;
            UiEvent::validate_playback_snapshot(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::PlaybackSnapshot(value))
        }
        "playback.finished" => {
            let value: PlaybackFinishedPayload = decode(name, payload)?;
            UiEvent::validate_playback_finished(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::PlaybackFinished(value))
        }
        "playback.failed" => {
            let value: PlaybackFailedPayload = decode(name, payload)?;
            UiEvent::validate_playback_failed(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::PlaybackFailed(value))
        }
        "diagnostics.snapshot" => {
            let value: DiagnosticsSnapshotDto = decode(name, payload)?;
            UiEvent::validate_diagnostics_snapshot(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::DiagnosticsSnapshot(value))
        }
        "calibration.progress" => {
            let value: CalibrationProgressPayload = decode(name, payload)?;
            UiEvent::validate_calibration_progress(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::CalibrationProgress(value))
        }
        "calibration.finished" => {
            let value: CalibrationFinishedPayload = decode(name, payload)?;
            UiEvent::validate_calibration_finished(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::CalibrationFinished(value))
        }
        "update.available" => {
            let value: UpdateAvailablePayload = decode(name, payload)?;
            UiEvent::validate_update_available(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::UpdateAvailable(value))
        }
        "update.result" => {
            let value: UpdateResultPayload = decode(name, payload)?;
            UiEvent::validate_update_result(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::UpdateResult(value))
        }
        "update.handoff_ready" => {
            let value: UpdateHandoffReadyPayload = decode(name, payload)?;
            UiEvent::validate_update_handoff(&value)
                .map_err(|error| ProtocolError::Invalid(error.to_string()))?;
            Ok(CoreEvent::UpdateHandoffReady(value))
        }
        other => Err(ProtocolError::Invalid(format!(
            "unsupported event name: {other}"
        ))),
    }
}

pub fn parse_message(frame: &[u8]) -> Result<CoreMessage, ProtocolError> {
    if frame.len() > MAX_OUTBOUND_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge(MAX_OUTBOUND_FRAME_BYTES));
    }
    let object = parse_object(frame)?;
    if object.get("v").and_then(Value::as_u64) != Some(DESKTOP_PROTOCOL_VERSION) {
        return Err(ProtocolError::Invalid(
            "unsupported protocol version".into(),
        ));
    }
    match object.get("type").and_then(Value::as_str) {
        Some("response") => {
            ensure_fields(&object, &["v", "id", "type", "ok", "result", "error"])?;
            let id = required_u64(&object, "id")?;
            if id > MAX_REQUEST_ID {
                return Err(ProtocolError::Invalid(
                    "response id is outside the JSON-safe range".into(),
                ));
            }
            let ok = object
                .get("ok")
                .and_then(Value::as_bool)
                .ok_or_else(|| ProtocolError::Invalid("response ok must be boolean".into()))?;
            if ok {
                if object.contains_key("error") {
                    return Err(ProtocolError::Invalid(
                        "successful response cannot contain error".into(),
                    ));
                }
                let result = object.get("result").cloned().ok_or_else(|| {
                    ProtocolError::Invalid("successful response lacks result".into())
                })?;
                Ok(CoreMessage::Response(CoreResponse {
                    id,
                    ok,
                    result: Some(result),
                    error: None,
                }))
            } else {
                if object.contains_key("result") {
                    return Err(ProtocolError::Invalid(
                        "failed response cannot contain result".into(),
                    ));
                }
                let error = object
                    .get("error")
                    .and_then(Value::as_object)
                    .ok_or_else(|| ProtocolError::Invalid("failed response lacks error".into()))?;
                ensure_fields(error, &["code", "message"])?;
                Ok(CoreMessage::Response(CoreResponse {
                    id,
                    ok,
                    result: None,
                    error: Some(CoreErrorPayload {
                        code: required_string(error, "code")?,
                        message: required_string(error, "message")?,
                    }),
                }))
            }
        }
        Some("event") => {
            ensure_fields(&object, &["v", "type", "name", "payload"])?;
            let name = required_string(&object, "name")?;
            let payload = object
                .get("payload")
                .cloned()
                .ok_or_else(|| ProtocolError::Invalid("event lacks payload".into()))?;
            Ok(CoreMessage::Event(parse_event(&name, payload)?))
        }
        Some(other) => Err(ProtocolError::Invalid(format!(
            "unsupported message type: {other}"
        ))),
        None => Err(ProtocolError::Invalid("message type is missing".into())),
    }
}

pub struct BoundedFrameReader<R> {
    reader: R,
    pending: Vec<u8>,
}

impl<R: Read> BoundedFrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
        }
    }

    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, ProtocolError> {
        loop {
            if let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
                let mut frame = self.pending.drain(..=index).collect::<Vec<_>>();
                frame.pop();
                if frame.last() == Some(&b'\r') {
                    frame.pop();
                }
                if frame.len() > MAX_OUTBOUND_FRAME_BYTES {
                    return Err(ProtocolError::FrameTooLarge(MAX_OUTBOUND_FRAME_BYTES));
                }
                return Ok(Some(frame));
            }
            if self.pending.len() > MAX_OUTBOUND_FRAME_BYTES {
                return Err(ProtocolError::FrameTooLarge(MAX_OUTBOUND_FRAME_BYTES));
            }
            let mut chunk = [0_u8; READ_CHUNK_BYTES];
            let read = self.reader.read(&mut chunk)?;
            if read == 0 {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(std::mem::take(&mut self.pending)));
            }
            self.pending.extend_from_slice(&chunk[..read]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn ready_payload() -> Value {
        serde_json::json!({
            "app_version": "fake-core",
            "protocol_version": 1,
            "native_build": {
                "native_build_commit": "a".repeat(40),
                "native_version": "3.5.0",
                "schema_version": 10,
                "native_abi": "cp314t-win_amd64",
                "rustc_version": "1.98.0",
                "win32_backend": true
            }
        })
    }

    fn event_frame(name: &str, payload: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "type": "event",
            "name": name,
            "payload": payload
        }))
        .expect("event frame")
    }

    #[test]
    fn parser_rejects_duplicate_keys_and_bad_version() {
        let duplicate =
            br#"{"v":1,"type":"event","name":"core.ready","name":"again","payload":{}}"#;
        assert!(matches!(
            parse_message(duplicate),
            Err(ProtocolError::Json(_))
        ));
        let bad_version = br#"{"v":2,"type":"event","name":"core.ready","payload":{}}"#;
        assert!(matches!(
            parse_message(bad_version),
            Err(ProtocolError::Invalid(_))
        ));

        let nested_duplicate =
            br#"{"v":1,"id":1,"type":"response","ok":false,"error":{"code":"x","code":"y","message":"bad"}}"#;
        assert!(matches!(
            parse_message(nested_duplicate),
            Err(ProtocolError::Json(_))
        ));

        let unexpected = br#"{"v":1,"id":1,"type":"response","ok":true,"result":{},"extra":1}"#;
        assert!(matches!(
            parse_message(unexpected),
            Err(ProtocolError::Invalid(_))
        ));

        let mixed_response =
            br#"{"v":1,"id":1,"type":"response","ok":true,"result":{},"error":{}}"#;
        assert!(matches!(
            parse_message(mixed_response),
            Err(ProtocolError::Invalid(_))
        ));
    }

    #[test]
    fn parser_accepts_only_typed_bounded_events() {
        assert!(matches!(
            parse_message(&event_frame("core.ready", ready_payload())),
            Ok(CoreMessage::Event(CoreEvent::Ready(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "core.fatal",
                serde_json::json!({"code": "failure", "message": "bounded"})
            )),
            Ok(CoreMessage::Event(CoreEvent::Fatal(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "catalog.changed",
                serde_json::json!({"generation": 2, "total": 500})
            )),
            Ok(CoreMessage::Event(CoreEvent::CatalogChanged(_)))
        ));
        let playback_ids = serde_json::json!({
            "session_id": "b".repeat(32),
            "song_id": "c".repeat(32),
        });
        assert!(matches!(
            parse_message(&event_frame(
                "playback.state_changed",
                serde_json::json!({
                    "session_id": playback_ids["session_id"],
                    "song_id": playback_ids["song_id"],
                    "state": "playing",
                    "physical": false,
                    "message": null,
                    "outcome": null
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::PlaybackStateChanged(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "playback.snapshot",
                serde_json::json!({
                    "session_id": playback_ids["session_id"],
                    "seq": 1,
                    "state": "playing",
                    "song_id": playback_ids["song_id"],
                    "title": "Aurora",
                    "current_us": 10,
                    "total_us": 100,
                    "pre_roll_remaining_us": 0,
                    "focus_state": "focused",
                    "health": "healthy",
                    "input_path_degraded": false,
                    "message": null
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::PlaybackSnapshot(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "playback.finished",
                serde_json::json!({
                    "session_id": playback_ids["session_id"],
                    "song_id": playback_ids["song_id"],
                    "outcome": "finished",
                    "total_us": 100,
                    "message": "done"
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::PlaybackFinished(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "playback.failed",
                serde_json::json!({
                    "session_id": playback_ids["session_id"],
                    "song_id": playback_ids["song_id"],
                    "code": "native_error",
                    "message": "failed"
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::PlaybackFailed(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "diagnostics.snapshot",
                serde_json::json!({
                    "seq": 1,
                    "max_lateness_us": 120,
                    "p50_ms": 0.4,
                    "p95_ms": 1.2,
                    "sigma_onset_ms": 0.2,
                    "late_2ms": 0,
                    "late_5ms": 0,
                    "late_10ms": 0,
                    "active_keys": 0,
                    "stuck_keys": 0,
                    "keys_dropped": 0,
                    "chord_split_events": 0,
                    "backend_status": "healthy",
                    "release_max_us": null,
                    "release_late_2ms": null,
                    "session_id": null
                }),
            )),
            Ok(CoreMessage::Event(CoreEvent::DiagnosticsSnapshot(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "calibration.progress",
                serde_json::json!({
                    "operation_id": "d".repeat(32),
                    "state": "running",
                    "phase": "measuring",
                    "completed": 1,
                    "total": 2,
                    "message": "sample"
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::CalibrationProgress(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "calibration.finished",
                serde_json::json!({
                    "operation_id": "d".repeat(32),
                    "outcome": "succeeded",
                    "status": "ready",
                    "margin_us": 800,
                    "sample_count": 12,
                    "source": "native",
                    "message": "complete",
                    "applied": true
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::CalibrationFinished(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "update.available",
                serde_json::json!({
                    "current_version": "3.5.0",
                    "available_version": "3.6.0",
                    "channel": "stable",
                    "release_notes": "release",
                    "published_at": "2026-08-30T00:00:00Z"
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::UpdateAvailable(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "update.result",
                serde_json::json!({
                    "state": "current",
                    "current_version": "3.5.0",
                    "available_version": null,
                    "channel": "stable",
                    "error": null
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::UpdateResult(_)))
        ));
        assert!(matches!(
            parse_message(&event_frame(
                "update.handoff_ready",
                serde_json::json!({
                    "handoff_id": "e".repeat(32),
                    "target_version": "3.6.0"
                })
            )),
            Ok(CoreMessage::Event(CoreEvent::UpdateHandoffReady(_)))
        ));
    }

    #[test]
    fn parser_rejects_unknown_or_malformed_events_fail_closed() {
        let cases = [
            event_frame("catalog.unknown", serde_json::json!({})),
            event_frame("catalog.changed", serde_json::json!({"generation": 2})),
            event_frame(
                "catalog.changed",
                serde_json::json!({"generation": "two", "total": 500}),
            ),
            event_frame(
                "catalog.changed",
                serde_json::json!({"generation": 2, "total": 500, "extra": true}),
            ),
            event_frame(
                "core.fatal",
                serde_json::json!({"code": "failure", "message": "bad", "extra": true}),
            ),
            event_frame(
                "core.ready",
                serde_json::json!({
                    "app_version": "fake-core",
                    "protocol_version": 1,
                    "native_build": {
                        "native_build_commit": "a".repeat(40),
                        "native_version": "3.5.0",
                        "schema_version": 10,
                        "native_abi": "cp314t-win_amd64",
                        "rustc_version": "1.98.0",
                        "win32_backend": "true"
                    }
                }),
            ),
            event_frame(
                "playback.state_changed",
                serde_json::json!({
                    "session_id": "b".repeat(32),
                    "song_id": "c".repeat(32),
                    "state": "unknown",
                    "physical": false,
                    "message": null,
                    "outcome": null
                }),
            ),
            event_frame(
                "playback.snapshot",
                serde_json::json!({
                    "session_id": "b".repeat(32),
                    "seq": 1,
                    "state": "playing",
                    "song_id": "c".repeat(32),
                    "title": "Aurora",
                    "current_us": 10,
                    "total_us": 100,
                    "pre_roll_remaining_us": 0,
                    "focus_state": "focused",
                    "health": "healthy",
                    "input_path_degraded": false,
                    "message": null,
                    "extra": true
                }),
            ),
            event_frame(
                "diagnostics.snapshot",
                serde_json::json!({
                    "seq": 1,
                    "max_lateness_us": 1,
                    "p50_ms": "bad",
                    "p95_ms": 1.0,
                    "sigma_onset_ms": 0.1,
                    "late_2ms": 0,
                    "late_5ms": 0,
                    "late_10ms": 0,
                    "active_keys": 0,
                    "stuck_keys": 0,
                    "keys_dropped": 0,
                    "chord_split_events": 0,
                    "backend_status": "healthy",
                    "release_max_us": null,
                    "release_late_2ms": null,
                    "session_id": null
                }),
            ),
            event_frame(
                "calibration.progress",
                serde_json::json!({
                    "operation_id": "d".repeat(32),
                    "state": "running",
                    "phase": "measure",
                    "completed": 3,
                    "total": 2,
                    "message": "bad"
                }),
            ),
            event_frame(
                "calibration.finished",
                serde_json::json!({
                    "operation_id": "d".repeat(32),
                    "outcome": "succeeded",
                    "status": "ready",
                    "sample_count": 1,
                    "source": "native",
                    "message": "done",
                    "applied": true,
                    "extra": true
                }),
            ),
            event_frame(
                "update.result",
                serde_json::json!({
                    "state": "current",
                    "current_version": "3.5.0",
                    "available_version": null,
                    "channel": "stable",
                    "error": null,
                    "extra": true
                }),
            ),
            event_frame(
                "update.handoff_ready",
                serde_json::json!({
                    "handoff_id": "e".repeat(32),
                    "target_version": 3.6
                }),
            ),
        ];
        for frame in cases {
            assert!(
                parse_message(&frame).is_err(),
                "malformed event was accepted: {}",
                String::from_utf8_lossy(&frame)
            );
        }
    }

    #[test]
    fn bounded_reader_reads_chunks_and_rejects_oversized_output() {
        let data = br#"{"v":1,"type":"event","name":"core.ready","payload":{"app_version":"fake","protocol_version":1,"native_build":{"native_build_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","native_version":"3.5.0","schema_version":10,"native_abi":"cp314t-win_amd64","rustc_version":"1.98.0","win32_backend":true}}}
{"v":1,"type":"event","name":"core.fatal","payload":{"code":"fake","message":"failure"}}"#;
        let mut reader = BoundedFrameReader::new(Cursor::new(data));
        assert!(reader.next_frame().unwrap().is_some());
        assert!(reader.next_frame().unwrap().is_some());
        assert!(reader.next_frame().unwrap().is_none());

        let mut reader =
            BoundedFrameReader::new(Cursor::new(vec![b'x'; MAX_OUTBOUND_FRAME_BYTES + 1]));
        assert!(matches!(
            reader.next_frame(),
            Err(ProtocolError::FrameTooLarge(_))
        ));
    }
}

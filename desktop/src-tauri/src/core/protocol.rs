use serde::de::{self, DeserializeSeed, MapAccess, Visitor};
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
pub struct CoreEvent {
    pub name: String,
    pub payload: Value,
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

struct StrictObject;

impl<'de> DeserializeSeed<'de> for StrictObject {
    type Value = Map<String, Value>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ObjectVisitor)
    }
}

struct ObjectVisitor;

impl<'de> Visitor<'de> for ObjectVisitor {
    type Value = Map<String, Value>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object with unique keys")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some((key, value)) = access.next_entry::<String, Value>()? {
            if object.insert(key.clone(), value).is_some() {
                return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
            }
        }
        Ok(object)
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
    let object = StrictObject
        .deserialize(&mut deserializer)
        .map_err(|error| ProtocolError::Json(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| ProtocolError::Json(error.to_string()))?;
    Ok(object)
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
                let error = object
                    .get("error")
                    .and_then(Value::as_object)
                    .ok_or_else(|| ProtocolError::Invalid("failed response lacks error".into()))?;
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
        Some("event") => Ok(CoreMessage::Event(CoreEvent {
            name: required_string(&object, "name")?,
            payload: object
                .get("payload")
                .cloned()
                .ok_or_else(|| ProtocolError::Invalid("event lacks payload".into()))?,
        })),
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
    }

    #[test]
    fn bounded_reader_reads_chunks_and_rejects_oversized_output() {
        let data = br#"{"v":1,"type":"event","name":"core.ready","payload":{}}
{"v":1,"type":"event","name":"core.fatal","payload":{}}"#;
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

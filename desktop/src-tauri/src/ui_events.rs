use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct UiEvent {
    pub v: u64,
    pub name: String,
    pub payload: Value,
}

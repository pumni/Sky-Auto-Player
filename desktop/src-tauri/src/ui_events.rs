use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct UiEvent {
    pub v: u64,
    pub name: String,
    pub payload: Value,
}

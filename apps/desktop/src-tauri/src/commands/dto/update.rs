use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ui_events::UpdateChannel;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdatePreferencesPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<UpdateChannel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_version: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdateBeginHandoffRequest {
    pub target_version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdatePreferencesDto {
    pub auto_check: bool,
    pub channel: UpdateChannel,
    pub skip_version: String,
    pub check_interval_s: i64,
    pub last_check_ts: i64,
    pub last_error_ts: i64,
}

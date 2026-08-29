use serde::{Deserialize, Serialize};
use ts_rs::TS;

const MAX_EVENT_TEXT_BYTES: usize = 4096;

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CoreReadyPayload {
    pub app_version: String,
    pub protocol_version: u64,
    pub native_build: NativeBuildPayload,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct NativeBuildPayload {
    pub native_build_commit: String,
    pub native_version: String,
    pub schema_version: u64,
    pub native_abi: String,
    pub rustc_version: String,
    pub win32_backend: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CoreFatalPayload {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CatalogChangedPayload {
    pub generation: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(tag = "name")]
pub enum UiEvent {
    #[serde(rename = "core.ready")]
    CoreReady { v: u64, payload: CoreReadyPayload },
    #[serde(rename = "core.fatal")]
    CoreFatal { v: u64, payload: CoreFatalPayload },
    #[serde(rename = "catalog.changed")]
    CatalogChanged {
        v: u64,
        payload: CatalogChangedPayload,
    },
}

impl UiEvent {
    pub(crate) fn validate_ready(payload: &CoreReadyPayload) -> Result<(), String> {
        validate_text("app_version", &payload.app_version)?;
        validate_text(
            "native_build_commit",
            &payload.native_build.native_build_commit,
        )?;
        validate_text("native_version", &payload.native_build.native_version)?;
        validate_text("native_abi", &payload.native_build.native_abi)?;
        validate_text("rustc_version", &payload.native_build.rustc_version)
    }

    pub(crate) fn validate_fatal(payload: &CoreFatalPayload) -> Result<(), String> {
        validate_text("code", &payload.code)?;
        validate_text("message", &payload.message)
    }

    pub(crate) fn validate_catalog_changed(payload: &CatalogChangedPayload) -> Result<(), String> {
        if payload.generation == 0 {
            return Err("event generation must be positive".into());
        }
        if payload.total > 10_000_000 {
            return Err("event catalog total exceeds the bounded contract".into());
        }
        Ok(())
    }
}

fn validate_text(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_EVENT_TEXT_BYTES || value.contains('\0') {
        return Err(format!("event {name} is outside the bounded text contract"));
    }
    Ok(())
}

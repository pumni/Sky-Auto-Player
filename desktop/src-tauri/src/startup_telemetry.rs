use serde_json::{Map, Value, json};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const TELEMETRY_ENV: &str = "SKY_STARTUP_TELEMETRY_PATH";
const SCHEMA_VERSION: u64 = 1;

struct StartupTelemetry {
    path: PathBuf,
    started_at: Instant,
    file: Mutex<Option<File>>,
}

static TELEMETRY: OnceLock<Option<StartupTelemetry>> = OnceLock::new();

pub(crate) fn initialize() {
    let _ = telemetry();
}

pub(crate) fn enabled() -> bool {
    telemetry().is_some()
}

pub(crate) fn record(marker: &str) {
    record_fields(marker, None);
}

pub(crate) fn record_counters(
    marker: &str,
    directories_visited: u64,
    files_visited: u64,
    supported_files: u64,
    duration_ms: Option<u64>,
) {
    let mut fields = Map::new();
    fields.insert("directories_visited".into(), json!(directories_visited));
    fields.insert("files_visited".into(), json!(files_visited));
    fields.insert("supported_files".into(), json!(supported_files));
    if let Some(duration_ms) = duration_ms {
        fields.insert("duration_ms".into(), json!(duration_ms));
    }
    record_fields(marker, Some(fields));
}

pub(crate) fn record_frontend_marker(marker: &str) -> Result<(), String> {
    if !matches!(
        marker,
        "frontend.entry" | "react.initialize.start" | "react.shell_ready" | "react.catalog_ready"
    ) {
        return Err("invalid startup telemetry marker".into());
    }
    record(marker);
    Ok(())
}

fn telemetry() -> Option<&'static StartupTelemetry> {
    TELEMETRY
        .get_or_init(|| {
            std::env::var_os(TELEMETRY_ENV).map(|path| StartupTelemetry {
                path: PathBuf::from(path),
                started_at: Instant::now(),
                file: Mutex::new(None),
            })
        })
        .as_ref()
}

fn record_fields(marker: &str, fields: Option<Map<String, Value>>) {
    let Some(telemetry) = telemetry() else {
        return;
    };
    let elapsed_us = telemetry
        .started_at
        .elapsed()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64;
    let mut event = Map::new();
    event.insert("schema_version".into(), json!(SCHEMA_VERSION));
    event.insert("marker".into(), json!(marker));
    event.insert("elapsed_us".into(), json!(elapsed_us));
    if let Some(fields) = fields {
        event.extend(fields);
    }
    let Ok(mut file) = telemetry.file.lock() else {
        return;
    };
    if file.is_none() {
        *file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&telemetry.path)
            .ok();
    }
    let Some(file) = file.as_mut() else {
        return;
    };
    if serde_json::to_writer(&mut *file, &Value::Object(event)).is_ok() {
        let _ = file.write_all(b"\n");
        let _ = file.flush();
    }
}

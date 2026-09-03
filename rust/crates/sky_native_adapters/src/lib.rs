//! Outer adapters for Wave 2 application services.
//!
//! This crate owns filesystem enumeration and persisted config compatibility.
//! It does not contain Tauri delivery policy, Python transport, realtime
//! playback, or updater transaction logic.

use serde_json::{Map, Value};
use sky_app_core::catalog::{
    CatalogError, CatalogSourceEntry, SUPPORTED_EXTENSIONS, SongSource, song_id_for_canonical_path,
};
use sky_app_core::library::{
    ImportedSourceKind, ImportedSourceRef, LIBRARY_MANIFEST_VERSION, LibraryError,
    LibraryManifestStore, LibraryManifestV1, LikedSongs,
};
use sky_app_core::settings::{
    ApplicationSettings, DEFAULT_GAME_FPS, DEFAULT_HOLD_FRAMES, DEFAULT_PROCESS_NAMES,
    DEFAULT_SONGS_DIR, DEFAULT_UPDATE_INTERVAL_S, HOLD_FRAME_OPTIONS, HotkeySettings,
    SafetySettings, SettingsError, SettingsStore, UpdateChannel, UpdatePreferences, VALID_FPS,
    normalize_settings,
};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub mod paths;
pub use paths::{AppPaths, CALIBRATION_EXE, V4_APP_IDENTIFIER, snapshot_directory};

pub const DEFAULT_TRANSPORT_MARGIN_US: u64 = 300;
pub const CALIBRATION_MARGIN_SOURCE_DEFAULT: &str = "default_transport_300";
pub const CALIBRATION_MARGIN_SOURCE_DEVICE: &str = "device_cache";
pub const CALIBRATION_MARGIN_SOURCE_INVALID: &str = "invalid_cache_transport_300";
pub const CALIBRATION_MARGIN_SOURCE_INCOMPATIBLE: &str = "incompatible_host_transport_300";
pub const CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE: &str = "out_of_envelope_transport_300";

pub const CALIBRATION_CACHE_VERSION: u64 = 8;
pub const CALIBRATION_EVIDENCE_KIND: &str = "sender_completion_hold_shrink";
pub const CALIBRATION_ARTIFACT_SCHEMA_VERSION: u64 = 11;
pub const CALIBRATION_NATIVE_VERSION: u64 = 15;
pub const CALIBRATION_MEASUREMENT_PROTOCOL_VERSION: u64 = 10;
pub const CALIBRATION_SOURCE_FORMULA_VERSION: u64 = 6;
pub const CALIBRATION_HOST_FINGERPRINT_VERSION: u64 = 2;
pub const CALIBRATION_SAMPLE_COUNT: u64 = 100;
pub const CALIBRATION_MAX_SHRINK_US: i64 = 100_000;
pub const CALIBRATION_REQUIRED_BUCKETS: [&str; 6] =
    ["1/hot", "1/cold", "5/hot", "5/cold", "15/hot", "15/cold"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationResolution {
    pub margin_us: u64,
    pub source: String,
}

/// Read the publishable sender-calibration result using the same fail-closed
/// categories as the Python loader.  The cache is evidence, not a source of
/// arbitrary timing values: only the current schema/formula/constants and a
/// valid applied margin are accepted.
pub fn load_calibration_resolution(path: impl AsRef<Path>) -> CalibrationResolution {
    let fallback = |source: &str| CalibrationResolution {
        margin_us: DEFAULT_TRANSPORT_MARGIN_US,
        source: source.into(),
    };
    let Ok(text) = fs::read_to_string(path) else {
        return fallback(CALIBRATION_MARGIN_SOURCE_DEFAULT);
    };
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(&text) else {
        return fallback(CALIBRATION_MARGIN_SOURCE_INVALID);
    };
    if root.get("version").and_then(Value::as_u64) != Some(CALIBRATION_CACHE_VERSION)
        || root.get("evidence_kind").and_then(Value::as_str) != Some(CALIBRATION_EVIDENCE_KIND)
        || root.get("artifact_schema_version").and_then(Value::as_u64)
            != Some(CALIBRATION_ARTIFACT_SCHEMA_VERSION)
        || root
            .get("native_calibration_version")
            .and_then(Value::as_u64)
            != Some(CALIBRATION_NATIVE_VERSION)
        || root
            .get("measurement_protocol_version")
            .and_then(Value::as_u64)
            != Some(CALIBRATION_MEASUREMENT_PROTOCOL_VERSION)
        || root.get("source").and_then(Value::as_str) != Some("device_cache")
        || root.get("source_formula_version").and_then(Value::as_u64)
            != Some(CALIBRATION_SOURCE_FORMULA_VERSION)
    {
        return fallback(CALIBRATION_MARGIN_SOURCE_INCOMPATIBLE);
    }
    let Some(qualification) = root.get("qualification").and_then(Value::as_object) else {
        return fallback(CALIBRATION_MARGIN_SOURCE_INVALID);
    };
    let Some(host) = valid_host_fingerprint(root.get("host_fingerprint")) else {
        return fallback(CALIBRATION_MARGIN_SOURCE_INVALID);
    };
    if !valid_provenance(&root) || !valid_scheduling_aids(root.get("scheduling_aids")) {
        return fallback(CALIBRATION_MARGIN_SOURCE_INVALID);
    }
    let Some((global_transport, worst_bucket)) = valid_pair_buckets(&root) else {
        return fallback(CALIBRATION_MARGIN_SOURCE_INVALID);
    };
    let (expected_status, candidate, applied) = qualification_values(global_transport);
    let status = qualification
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| root.get("status").and_then(Value::as_str));
    if status == Some("out_of_envelope") {
        if !qualification_matches(
            &root,
            qualification,
            QualificationValues {
                status,
                worst_bucket: &worst_bucket,
                global_transport,
                candidate,
                applied,
                expected_status,
            },
        ) {
            return fallback(CALIBRATION_MARGIN_SOURCE_INVALID);
        }
        return fallback(CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE);
    }
    if !qualification_matches(
        &root,
        qualification,
        QualificationValues {
            status,
            worst_bucket: &worst_bucket,
            global_transport,
            candidate,
            applied,
            expected_status,
        },
    ) || status != Some("valid")
    {
        return fallback(CALIBRATION_MARGIN_SOURCE_INVALID);
    }
    let margin = applied.expect("valid qualification has an applied margin");
    #[cfg(windows)]
    if !host_matches_current(host) {
        return fallback(CALIBRATION_MARGIN_SOURCE_INCOMPATIBLE);
    }
    CalibrationResolution {
        margin_us: margin,
        source: CALIBRATION_MARGIN_SOURCE_DEVICE.into(),
    }
}

fn object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value?.as_object()
}

fn unsigned(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key)?.as_u64()
}

fn signed(object: &Map<String, Value>, key: &str) -> Option<i64> {
    object.get(key)?.as_i64()
}

fn nonempty_string(object: &Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty() && value != "unknown")
}

fn valid_host_fingerprint(value: Option<&Value>) -> Option<&Map<String, Value>> {
    let host = object(value)?;
    if unsigned(host, "host_fingerprint_version") != Some(CALIBRATION_HOST_FINGERPRINT_VERSION)
        || unsigned(host, "qpc_frequency_hz").is_none()
        || !nonempty_string(host, "win32_build")
        || !nonempty_string(host, "processor_architecture")
        || !nonempty_string(host, "cpu_vendor")
        || [
            "cpu_family",
            "cpu_model",
            "cpu_stepping",
            "logical_processor_count",
            "processor_group_count",
        ]
        .iter()
        .any(|key| unsigned(host, key).is_none())
    {
        return None;
    }
    let efficiency = host.get("cpu_set_efficiency_classes")?.as_array()?;
    if efficiency.iter().any(|value| value.as_u64().is_none()) {
        return None;
    }
    for key in ["highest_efficiency_class", "lowest_efficiency_class"] {
        if host
            .get(key)
            .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
        {
            return None;
        }
    }
    if host
        .get("sampled_at_us")
        .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
    {
        return None;
    }
    Some(host)
}

fn valid_provenance(root: &Map<String, Value>) -> bool {
    [
        "source_git_sha",
        "native_build_id",
        "native_source_fingerprint",
        "rustc_version",
    ]
    .iter()
    .all(|key| nonempty_string(root, key))
        && root.get("source_git_sha") == root.get("native_build_id")
        && root.get("dirty_worktree") == Some(&Value::Bool(false))
}

fn valid_scheduling_aids(value: Option<&Value>) -> bool {
    let Some(aids) = object(value) else {
        return false;
    };
    let Some(mmcss) = aids.get("mmcss_acquired").and_then(Value::as_str) else {
        return false;
    };
    let Some(mmcss_active) = aids.get("mmcss_active").and_then(Value::as_bool) else {
        return false;
    };
    let Some(_power_active) = aids.get("power_throttling_active").and_then(Value::as_bool) else {
        return false;
    };
    let Some(waiter_mode) = aids.get("waiter_mode").and_then(Value::as_str) else {
        return false;
    };
    [
        "off",
        "mmcss:Games",
        "thread:highest",
        "thread:time_critical",
    ]
    .contains(&mmcss)
        && mmcss_active == (mmcss != "off")
        && [
            "event+high_resolution_timer",
            "high_resolution_timer",
            "event+timer_resolution_fallback",
            "timer_resolution_fallback",
        ]
        .contains(&waiter_mode)
        && ["off", "mmcss:Games", "thread:highest"].contains(&mmcss)
        && waiter_mode == "event+high_resolution_timer"
}

fn valid_signed_quantiles(value: Option<&Value>, unsigned_values: bool) -> bool {
    let Some(values) = object(value) else {
        return false;
    };
    let fields = ["min", "p50", "p90", "p95", "p99", "max", "mean"];
    let Some(parsed) = fields
        .iter()
        .map(|key| signed(values, key))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let ordered = &parsed[..6];
    ordered.windows(2).all(|pair| pair[0] <= pair[1])
        && parsed[0] <= parsed[6]
        && parsed[6] <= parsed[5]
        && ordered.iter().all(|value| {
            (unsigned_values && *value >= 0)
                || (!unsigned_values && value.abs() <= CALIBRATION_MAX_SHRINK_US)
        })
}

fn valid_pair_buckets(root: &Map<String, Value>) -> Option<(i64, String)> {
    let required = root.get("required_buckets")?.as_array()?;
    if required.len() != CALIBRATION_REQUIRED_BUCKETS.len()
        || required
            .iter()
            .zip(CALIBRATION_REQUIRED_BUCKETS)
            .any(|(actual, expected)| actual.as_str() != Some(expected))
    {
        return None;
    }
    let buckets = object(root.get("pair_buckets"))?;
    if buckets.len() != CALIBRATION_REQUIRED_BUCKETS.len()
        || CALIBRATION_REQUIRED_BUCKETS
            .iter()
            .any(|key| !buckets.contains_key(*key))
    {
        return None;
    }
    let mut global = 0_i64;
    let mut worst = CALIBRATION_REQUIRED_BUCKETS[0];
    for key in CALIBRATION_REQUIRED_BUCKETS {
        let bucket = object(buckets.get(key))?;
        let attempted = unsigned(bucket, "attempted")?;
        let clean = unsigned(bucket, "clean_pair_count")?;
        let rejected = unsigned(bucket, "rejected")?;
        if !(CALIBRATION_SAMPLE_COUNT..=CALIBRATION_SAMPLE_COUNT * 2).contains(&attempted)
            || clean != CALIBRATION_SAMPLE_COUNT
            || rejected != attempted.saturating_sub(clean)
            || unsigned(bucket, "anomaly_count")? != rejected
            || unsigned(bucket, "class_mismatch_count")? != rejected
            || unsigned(bucket, "timeout_count")? != 0
            || unsigned(bucket, "partial_send")? != 0
            || !valid_signed_quantiles(bucket.get("pair_sender_hold_shrink_us"), false)
            || !valid_signed_quantiles(bucket.get("scheduler_shrink_us"), false)
            || !valid_signed_quantiles(bucket.get("sendinput_shrink_us"), false)
            || !valid_signed_quantiles(bucket.get("down_call_duration_us"), true)
            || !valid_signed_quantiles(bucket.get("up_call_duration_us"), true)
        {
            return None;
        }
        let bucket_max = signed(object(bucket.get("sendinput_shrink_us"))?, "max")?;
        if bucket_max > global {
            global = bucket_max;
            worst = key;
        }
    }
    Some((global.max(0), worst.into()))
}

fn qualification_values(global_transport: i64) -> (&'static str, u64, Option<u64>) {
    let candidate = global_transport as u64 + 100;
    if candidate > 2_000 {
        ("out_of_envelope", candidate, None)
    } else {
        ("valid", candidate, Some(300_u64.max(candidate)))
    }
}

struct QualificationValues<'a> {
    status: Option<&'a str>,
    worst_bucket: &'a str,
    global_transport: i64,
    candidate: u64,
    applied: Option<u64>,
    expected_status: &'a str,
}

fn qualification_matches(
    root: &Map<String, Value>,
    qualification: &Map<String, Value>,
    values: QualificationValues<'_>,
) -> bool {
    let Some(worst_serialized) = qualification.get("worst_bucket").and_then(Value::as_str) else {
        return false;
    };
    qualification.get("basis").and_then(Value::as_str)
        == Some("max_required_bucket_max_positive_sendinput_shrink")
        && worst_serialized == values.worst_bucket
        && qualification
            .get("transport_worst_positive_us")
            .and_then(Value::as_i64)
            == Some(values.global_transport)
        && qualification.get("guard_us").and_then(Value::as_u64) == Some(100)
        && qualification.get("floor_us").and_then(Value::as_u64) == Some(300)
        && qualification.get("ceiling_us").and_then(Value::as_u64) == Some(2_000)
        && qualification
            .get("candidate_transport_margin_us")
            .and_then(Value::as_u64)
            == Some(values.candidate)
        && qualification
            .get("applied_transport_margin_us")
            .and_then(Value::as_u64)
            == values.applied
        && values.status == Some(values.expected_status)
        && root.get("status").and_then(Value::as_str) == values.status
        && root.get("transport_margin_us").and_then(Value::as_u64) == values.applied
        && root.get("transport_margin_source").and_then(Value::as_str)
            == Some(CALIBRATION_MARGIN_SOURCE_DEVICE)
        && root
            .get("transport_worst_positive_us")
            .and_then(Value::as_i64)
            == Some(values.global_transport)
        && root.get("transport_guard_us").and_then(Value::as_u64) == Some(100)
        && root.get("transport_floor_us").and_then(Value::as_u64) == Some(300)
        && root.get("transport_ceiling_us").and_then(Value::as_u64) == Some(2_000)
        && root
            .get("calibration_timing_qualified")
            .and_then(Value::as_bool)
            == Some(values.expected_status == "valid")
}

#[cfg(windows)]
fn host_matches_current(host: &Map<String, Value>) -> bool {
    let Ok(current) = sky_player::adapter_support::build_host_fingerprint() else {
        return false;
    };
    let Ok(current) = serde_json::to_value(current) else {
        return false;
    };
    let Some(current) = current.as_object() else {
        return false;
    };
    [
        "host_fingerprint_version",
        "qpc_frequency_hz",
        "win32_build",
        "processor_architecture",
        "cpu_vendor",
        "cpu_family",
        "cpu_model",
        "cpu_stepping",
        "logical_processor_count",
        "processor_group_count",
        "cpu_set_efficiency_classes",
        "highest_efficiency_class",
        "lowest_efficiency_class",
    ]
    .iter()
    .all(|key| host.get(*key) == current.get(*key))
}

#[derive(Clone)]
pub struct JsonSettingsStore {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl JsonSettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_object(&self) -> Map<String, Value> {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Map::new();
        };
        serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    }
}

impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Result<ApplicationSettings, SettingsError> {
        let raw = self.read_object();
        let migrated = migrate_raw(&raw);
        if migrated != raw {
            let encoded = serde_json::to_vec_pretty(&Value::Object(migrated.clone()))
                .map_err(|error| SettingsError::Storage(error.to_string()))?;
            atomic_replace(&self.path, &encoded)?;
        }
        Ok(settings_from_raw(&migrated))
    }

    fn save(&self, settings: &ApplicationSettings) -> Result<(), SettingsError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| SettingsError::Storage("settings write lock poisoned".into()))?;
        let mut raw = self.read_object();
        overlay_settings(&mut raw, settings);
        let encoded = serde_json::to_vec_pretty(&Value::Object(raw))
            .map_err(|error| SettingsError::Storage(error.to_string()))?;
        atomic_replace(&self.path, &encoded)
    }
}

/// Native-only persistence for user library data.  The canonical paths in
/// this document are never projected to the frontend; they are resolved only
/// while composing the native catalog.
#[derive(Clone)]
pub struct JsonLibraryManifestStore {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl JsonLibraryManifestStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl LibraryManifestStore for JsonLibraryManifestStore {
    fn load(&self) -> Result<LibraryManifestV1, LibraryError> {
        if !self.path.exists() {
            return Ok(LibraryManifestV1 {
                version: LIBRARY_MANIFEST_VERSION,
                ..Default::default()
            });
        }
        let text = fs::read_to_string(&self.path)
            .map_err(|error| LibraryError::Storage(error.to_string()))?;
        serde_json::from_str(&text).map_err(|error| LibraryError::Storage(error.to_string()))
    }

    fn save(&self, manifest: &LibraryManifestV1) -> Result<(), LibraryError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LibraryError::Storage("library manifest write lock poisoned".into()))?;
        let encoded = serde_json::to_vec_pretty(manifest)
            .map_err(|error| LibraryError::Storage(error.to_string()))?;
        atomic_replace(&self.path, &encoded)
            .map_err(|error| LibraryError::Storage(error.to_string()))
    }
}

pub struct FileCatalogSource {
    root: PathBuf,
}

/// Native-only catalog composition. The entries retain canonical paths for
/// local file access, while the membership/status projections are safe to
/// summarize across the IPC boundary.
#[derive(Debug, Clone, Default)]
pub struct CatalogComposition {
    pub entries: Vec<CatalogSourceEntry>,
    pub primary_membership: BTreeSet<String>,
    pub imported_membership: HashMap<String, BTreeSet<String>>,
    pub imported_status: Vec<ImportedSourceCatalogStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSourceCatalogStatus {
    pub source_id: String,
    pub kind: ImportedSourceKind,
    pub display_name: String,
    pub song_count: usize,
    pub available: bool,
}

impl FileCatalogSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn catalog_composition_with_imports(
        &self,
        imports: &[ImportedSourceRef],
    ) -> Result<CatalogComposition, CatalogError> {
        let mut composition = CatalogComposition {
            entries: self.entries()?,
            ..Default::default()
        };
        composition.primary_membership = composition
            .entries
            .iter()
            .map(|entry| song_id_for_canonical_path(&entry.canonical_path))
            .collect();

        for import in imports {
            let path = PathBuf::from(&import.canonical_path);
            let display_name = import_display_name(&path, import.kind);
            if !path.exists() {
                composition
                    .imported_status
                    .push(ImportedSourceCatalogStatus {
                        source_id: import.source_id.clone(),
                        kind: import.kind,
                        display_name,
                        song_count: 0,
                        available: false,
                    });
                composition
                    .imported_membership
                    .insert(import.source_id.clone(), BTreeSet::new());
                continue;
            }

            let imported = match import.kind {
                ImportedSourceKind::File => entries_from_file(&path),
                ImportedSourceKind::Folder => entries_from_directory(&path, true),
            };
            let imported = match imported {
                Ok(entries) => entries,
                Err(_) => {
                    composition
                        .imported_status
                        .push(ImportedSourceCatalogStatus {
                            source_id: import.source_id.clone(),
                            kind: import.kind,
                            display_name,
                            song_count: 0,
                            available: false,
                        });
                    composition
                        .imported_membership
                        .insert(import.source_id.clone(), BTreeSet::new());
                    continue;
                }
            };
            let membership = imported
                .iter()
                .map(|entry| song_id_for_canonical_path(&entry.canonical_path))
                .collect::<BTreeSet<_>>();
            composition
                .imported_status
                .push(ImportedSourceCatalogStatus {
                    source_id: import.source_id.clone(),
                    kind: import.kind,
                    display_name,
                    song_count: membership.len(),
                    available: true,
                });
            composition
                .imported_membership
                .insert(import.source_id.clone(), membership);
            composition.entries.extend(imported);
        }

        composition
            .entries
            .sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
        composition.entries.dedup_by(|left, right| {
            left.canonical_path
                .eq_ignore_ascii_case(&right.canonical_path)
        });
        Ok(composition)
    }

    /// Compose the primary songs directory with explicit native-owned import
    /// references. Missing imported paths are ignored so a disconnected
    /// removable drive does not make the rest of the catalog unavailable.
    pub fn entries_with_imports(
        &self,
        imports: &[ImportedSourceRef],
    ) -> Result<Vec<CatalogSourceEntry>, CatalogError> {
        Ok(self.catalog_composition_with_imports(imports)?.entries)
    }
}

impl SongSource for FileCatalogSource {
    fn entries(&self) -> Result<Vec<CatalogSourceEntry>, CatalogError> {
        entries_from_directory(&self.root, false)
    }
}

fn entries_from_file(path: &Path) -> Result<Vec<CatalogSourceEntry>, CatalogError> {
    if !path.is_file() || !is_supported(path) {
        return Ok(Vec::new());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
    Ok(vec![CatalogSourceEntry {
        canonical_path: canonical.to_string_lossy().into_owned(),
        title: path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned(),
    }])
}

fn entries_from_directory(
    root: &Path,
    recursive: bool,
) -> Result<Vec<CatalogSourceEntry>, CatalogError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        return Err(CatalogError::SourceUnavailable(
            "songs directory is not a directory".into(),
        ));
    }
    let mut entries = Vec::new();
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        let read_dir = fs::read_dir(&directory)
            .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
        for item in read_dir {
            let item = item.map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
            let path = item.path();
            let file_type = item
                .file_type()
                .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
            if recursive && file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file() {
                entries.extend(entries_from_file(&path)?);
            }
        }
        if !recursive {
            break;
        }
    }
    entries.sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
    Ok(entries)
}

fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn import_display_name(path: &Path, kind: ImportedSourceKind) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| match kind {
            ImportedSourceKind::File => "Imported file".to_owned(),
            ImportedSourceKind::Folder => "Imported folder".to_owned(),
        })
}

fn settings_from_raw(raw: &Map<String, Value>) -> ApplicationSettings {
    let mut settings = ApplicationSettings::default();
    settings.theme = raw_string(raw, "theme", &settings.theme);
    settings.ui_background_mode =
        raw_string(raw, "ui_background_mode", &settings.ui_background_mode);
    settings.playback_defaults.hold_frames =
        raw_f64(raw, "default_hold_frames", DEFAULT_HOLD_FRAMES);
    settings.playback_defaults.tempo_scale = raw_f64(raw, "default_tempo_scale", 1.0);
    settings.playback_defaults.fps = raw_fps(raw, "game_fps", DEFAULT_GAME_FPS);
    settings.telemetry_enabled = raw_bool(raw, "telemetry_enabled_by_default", false);
    settings.verbose_hud = raw_bool(raw, "verbose_hud", false);
    settings.songs_dir = raw_string(raw, "songs_dir", DEFAULT_SONGS_DIR);
    settings.allow_title_fallback = raw_bool(raw, "allow_title_fallback", false);
    settings.liked_songs =
        LikedSongs::from_persisted(raw_string_list(raw.get("liked_song_ids")).unwrap_or_default());
    settings.sky_process_names =
        raw_string_list(raw.get("sky_process_names")).unwrap_or_else(|| {
            DEFAULT_PROCESS_NAMES
                .iter()
                .map(|value| (*value).into())
                .collect()
        });

    if let Some(hotkeys) = raw.get("hotkeys").and_then(Value::as_object) {
        settings.hotkeys = HotkeySettings {
            pause: object_string(hotkeys, "pause", &settings.hotkeys.pause),
            skip: object_string(hotkeys, "skip", &settings.hotkeys.skip),
            quit: object_string(hotkeys, "quit", &settings.hotkeys.quit),
            refocus: object_string(hotkeys, "refocus", &settings.hotkeys.refocus),
            panic: object_string(hotkeys, "panic", &settings.hotkeys.panic),
        };
    }
    if let Some(safety) = raw.get("safety").and_then(Value::as_object) {
        settings.safety = SafetySettings {
            prompt_on_medium_risk: object_bool(safety, "prompt_on_medium_risk", true),
            prompt_on_high_risk: object_bool(safety, "prompt_on_high_risk", true),
        };
    }
    if let Some(update) = raw.get("update").and_then(Value::as_object) {
        settings.update = UpdatePreferences {
            auto_check: object_bool(update, "auto_check", true),
            channel: UpdateChannel::parse(&object_string(update, "channel", "stable"))
                .unwrap_or(UpdateChannel::Stable),
            skip_version: object_string_only(update, "skip_version", ""),
            check_interval_s: object_i64(update, "check_interval_s", DEFAULT_UPDATE_INTERVAL_S)
                .max(0),
            last_check_ts: object_i64(update, "last_check_ts", 0),
            last_error_ts: object_i64(update, "last_error_ts", 0),
            last_notified_version: object_string_only(update, "last_notified_version", ""),
            legacy_old_dir_sweep_pending: object_bool(
                update,
                "legacy_old_dir_sweep_pending",
                false,
            ) || update.contains_key("pending_update_version")
                || update.contains_key("auto_apply"),
        };
    }
    normalize_settings(settings)
}

fn migrate_raw(raw: &Map<String, Value>) -> Map<String, Value> {
    let mut migrated = raw.clone();
    let fps = resolve_fps(
        raw.get("game_fps")
            .and_then(python_int)
            .unwrap_or(i64::from(DEFAULT_GAME_FPS)),
    );
    let profile = raw
        .get("default_timing_profile")
        .map(python_string)
        .unwrap_or_else(|| "balanced".into())
        .to_lowercase()
        .replace('-', "_");
    let selected = selected_timing_profile(raw.get("timing_profiles"), &profile);

    let mut candidate = selected.and_then(|profile| {
        ["min_hold_frames", "hold_frames"]
            .into_iter()
            .find_map(|key| {
                profile
                    .get(key)
                    .and_then(legacy_float)
                    .and_then(nearest_hold_frames)
            })
    });
    if candidate.is_none() {
        let frame_us = (1_000_000_i64 + i64::from(fps) - 1) / i64::from(fps);
        candidate = selected.and_then(|profile| {
            ["min_hold_us", "hold_us"].into_iter().find_map(|key| {
                profile
                    .get(key)
                    .and_then(legacy_float)
                    .and_then(|value| nearest_hold_frames(value / frame_us as f64))
            })
        });
    }
    let candidate = candidate.unwrap_or_else(|| legacy_profile_hold(&profile));
    let hold = raw
        .get("default_hold_frames")
        .and_then(numeric_float)
        .and_then(nearest_supported_hold)
        .unwrap_or(DEFAULT_HOLD_FRAMES);
    migrated.insert("schema_version".into(), Value::from(3_u32));
    migrated.insert(
        "default_hold_frames".into(),
        Value::from(if raw.contains_key("default_hold_frames") {
            hold
        } else {
            candidate
        }),
    );
    for key in [
        "default_timing_profile",
        "timing_profiles",
        "frame_timing",
        "hold_us",
        "min_hold_us",
        "hold_frames",
        "min_hold_frames",
        "hold_unframed_us",
        "min_hold_unframed_us",
    ] {
        migrated.remove(key);
    }
    migrated
}

fn legacy_profile_hold(profile: &str) -> f64 {
    match profile {
        "audience_safe" | "remote_safe" | "online_audible_safe" | "online_audible" => 1.5,
        _ => DEFAULT_HOLD_FRAMES,
    }
}

fn selected_timing_profile<'a>(
    value: Option<&'a Value>,
    profile: &str,
) -> Option<&'a Map<String, Value>> {
    let profiles = value?.as_object()?;
    if let Some(selected) = profiles.get(profile) {
        if let Some(object) = selected.as_object() {
            if !object.is_empty() {
                return Some(object);
            }
        } else if !json_is_falsy(selected) {
            // Python keeps a truthy non-dict exact match and then discards it;
            // it does not continue looking for an alias in that case.
            return None;
        }
    }
    profiles
        .iter()
        .find(|(key, _)| key.to_lowercase().replace('-', "_") == profile)
        .and_then(|(_, selected)| selected.as_object())
}

fn json_is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(value) => !value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number == 0.0),
        Value::String(value) => value.is_empty(),
        Value::Array(value) => value.is_empty(),
        Value::Object(value) => value.is_empty(),
    }
}

fn raw_string(raw: &Map<String, Value>, key: &str, default: &str) -> String {
    raw.get(key)
        .map(python_string)
        .unwrap_or_else(|| default.into())
}

fn raw_bool(raw: &Map<String, Value>, key: &str, default: bool) -> bool {
    raw.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn raw_f64(raw: &Map<String, Value>, key: &str, default: f64) -> f64 {
    raw.get(key)
        .and_then(python_float)
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn raw_fps(raw: &Map<String, Value>, key: &str, default: u16) -> u16 {
    resolve_fps(
        raw.get(key)
            .and_then(python_int)
            .unwrap_or(i64::from(default)),
    )
}

fn raw_string_list(value: Option<&Value>) -> Option<Vec<String>> {
    value?
        .as_array()
        .map(|items| items.iter().map(python_string).collect())
}

fn object_string(object: &Map<String, Value>, key: &str, default: &str) -> String {
    object
        .get(key)
        .map(python_string)
        .unwrap_or_else(|| default.into())
}

fn object_string_only(object: &Map<String, Value>, key: &str, default: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned()
}

fn object_bool(object: &Map<String, Value>, key: &str, default: bool) -> bool {
    object.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn object_i64(object: &Map<String, Value>, key: &str, default: i64) -> i64 {
    object.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn python_string(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(value) => {
            if *value {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{}: {}", python_repr_string(key), python_repr(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::String(value) => python_repr_string(value),
        _ => python_string(value),
    }
}

fn python_repr_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn python_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(_) | Value::Null => None,
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn legacy_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => python_float(value),
    }
}

fn numeric_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(_) | Value::Null | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            None
        }
        Value::Number(value) => value.as_f64(),
    }
}

fn python_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(_) | Value::Null => None,
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| {
                value
                    .as_f64()
                    .map(|value| value.trunc())
                    .filter(|value| value.is_finite())
                    .and_then(|value| i64::try_from(value as i128).ok())
            }),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn nearest_hold_frames(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let mut best = HOLD_FRAME_OPTIONS[0];
    for option in HOLD_FRAME_OPTIONS {
        if (option - value).abs() <= (best - value).abs() {
            best = option;
        }
    }
    Some(best)
}

fn nearest_supported_hold(value: f64) -> Option<f64> {
    HOLD_FRAME_OPTIONS.contains(&value).then_some(value)
}

fn resolve_fps(value: i64) -> u16 {
    u16::try_from(value)
        .ok()
        .filter(|value| VALID_FPS.contains(value))
        .unwrap_or(DEFAULT_GAME_FPS)
}

fn overlay_settings(raw: &mut Map<String, Value>, settings: &ApplicationSettings) {
    raw.insert(
        "schema_version".into(),
        Value::from(sky_app_core::settings::SCHEMA_VERSION),
    );
    raw.insert("theme".into(), Value::from(settings.theme.clone()));
    raw.insert(
        "ui_background_mode".into(),
        Value::from(settings.ui_background_mode.clone()),
    );
    raw.insert(
        "default_hold_frames".into(),
        Value::from(settings.playback_defaults.hold_frames),
    );
    raw.insert(
        "default_tempo_scale".into(),
        Value::from(settings.playback_defaults.tempo_scale),
    );
    raw.insert(
        "game_fps".into(),
        Value::from(settings.playback_defaults.fps),
    );
    raw.insert(
        "telemetry_enabled_by_default".into(),
        Value::from(settings.telemetry_enabled),
    );
    raw.insert("verbose_hud".into(), Value::from(settings.verbose_hud));
    raw.insert("songs_dir".into(), Value::from(settings.songs_dir.clone()));
    raw.insert(
        "sky_process_names".into(),
        Value::Array(
            settings
                .sky_process_names
                .iter()
                .cloned()
                .map(Value::from)
                .collect(),
        ),
    );
    raw.insert(
        "allow_title_fallback".into(),
        Value::from(settings.allow_title_fallback),
    );
    if !settings.liked_songs.is_empty() || raw.contains_key("liked_song_ids") {
        raw.insert(
            "liked_song_ids".into(),
            Value::Array(
                settings
                    .liked_songs
                    .ids()
                    .iter()
                    .cloned()
                    .map(Value::from)
                    .collect(),
            ),
        );
    }
    raw.insert("hotkeys".into(), serde_json::json!({"pause": settings.hotkeys.pause, "skip": settings.hotkeys.skip, "quit": settings.hotkeys.quit, "refocus": settings.hotkeys.refocus, "panic": settings.hotkeys.panic}));
    raw.insert("safety".into(), serde_json::json!({"prompt_on_medium_risk": settings.safety.prompt_on_medium_risk, "prompt_on_high_risk": settings.safety.prompt_on_high_risk}));
    let mut update = raw
        .remove("update")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    update.insert("auto_check".into(), Value::from(settings.update.auto_check));
    update.insert(
        "channel".into(),
        Value::from(settings.update.channel.as_str()),
    );
    update.insert(
        "skip_version".into(),
        Value::from(settings.update.skip_version.clone()),
    );
    update.insert(
        "check_interval_s".into(),
        Value::from(settings.update.check_interval_s),
    );
    update.insert(
        "last_check_ts".into(),
        Value::from(settings.update.last_check_ts),
    );
    update.insert(
        "last_error_ts".into(),
        Value::from(settings.update.last_error_ts),
    );
    update.insert(
        "last_notified_version".into(),
        Value::from(settings.update.last_notified_version.clone()),
    );
    update.insert(
        "legacy_old_dir_sweep_pending".into(),
        Value::from(settings.update.legacy_old_dir_sweep_pending),
    );
    raw.insert("update".into(), Value::Object(update));
    for key in [
        "default_timing_profile",
        "timing_profiles",
        "frame_timing",
        "hold_us",
        "min_hold_us",
        "hold_frames",
        "min_hold_frames",
        "hold_unframed_us",
        "min_hold_unframed_us",
    ] {
        raw.remove(key);
    }
}

fn atomic_replace(path: &Path, contents: &[u8]) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SettingsError::Storage(error.to_string()))?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, contents).map_err(|error| SettingsError::Storage(error.to_string()))?;
    replace_file(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        SettingsError::Storage(error)
    })
}

#[cfg(windows)]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source = temp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    fs::rename(temp, target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_app_core::settings::{PlaybackDefaultsPatch, SettingsPatch, SettingsService};

    #[test]
    fn settings_store_preserves_unknown_keys_and_writes_atomically() {
        let root = std::env::temp_dir().join(format!("sky-w2-settings-{}", std::process::id()));
        let path = root.join("config.json");
        fs::create_dir_all(&root).expect("temp root");
        fs::write(
            &path,
            br#"{"theme":"aurora","future_field":42,"update":{"channel":"stable","future":true}}"#,
        )
        .expect("seed");
        let store = JsonSettingsStore::new(&path);
        let mut service = SettingsService::load(store.clone()).expect("load");
        service
            .patch(&SettingsPatch {
                playback_defaults: Some(PlaybackDefaultsPatch {
                    fps: Some(120),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .expect("patch");
        let raw: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(raw["future_field"], 42);
        assert_eq!(raw["update"]["future"], true);
        assert_eq!(raw["game_fps"], 120);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_source_handles_missing_directory_as_empty() {
        let source = FileCatalogSource::new(std::env::temp_dir().join("sky-w2-no-such-directory"));
        assert!(source.entries().expect("missing root").is_empty());
    }

    #[test]
    fn settings_store_reads_legacy_timing_profile_and_preserves_unknown_data() {
        let root =
            std::env::temp_dir().join(format!("sky-w2-legacy-settings-{}", std::process::id()));
        let path = root.join("config.json");
        fs::create_dir_all(&root).expect("temp root");
        fs::write(
            &path,
            br#"{"schema_version":2,"default_timing_profile":"audience-safe","timing_profiles":{"audience_safe":{"min_hold_frames":1.5}},"future":true}"#,
        )
        .expect("seed");
        let store = JsonSettingsStore::new(&path);
        let settings = store.load().expect("load legacy settings");
        assert_eq!(settings.playback_defaults.hold_frames, 1.5);
        store.save(&settings).expect("save migrated settings");
        let raw: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(raw["future"], true);
        assert!(raw.get("default_timing_profile").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn calibration_resolution_accepts_only_publishable_current_cache() {
        let root =
            std::env::temp_dir().join(format!("sky-w3-calibration-{}-{}", std::process::id(), 1));
        fs::create_dir_all(root.parent().expect("temp parent")).expect("temp parent");
        let path = root.with_extension("json");
        fs::write(&path, valid_calibration_cache(677).to_string()).expect("cache");
        assert_eq!(
            load_calibration_resolution(&path),
            CalibrationResolution {
                margin_us: 777,
                source: CALIBRATION_MARGIN_SOURCE_DEVICE.into()
            }
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn calibration_resolution_fails_closed_for_invalid_and_unhealthy_cache() {
        let path = std::env::temp_dir().join(format!(
            "sky-w3-invalid-calibration-{}.json",
            std::process::id()
        ));
        fs::write(&path, b"{not-json").expect("cache");
        assert_eq!(
            load_calibration_resolution(&path).source,
            CALIBRATION_MARGIN_SOURCE_INVALID
        );
        fs::write(&path, valid_calibration_cache(2_500).to_string()).expect("cache");
        assert_eq!(
            load_calibration_resolution(&path).source,
            CALIBRATION_MARGIN_SOURCE_OUT_OF_ENVELOPE
        );
        let _ = fs::remove_file(path);
    }

    fn quantiles(min: i64, max: i64) -> Value {
        serde_json::json!({
            "min": min,
            "p50": min,
            "p90": min.max(0),
            "p95": min.max(0),
            "p99": max,
            "max": max,
            "mean": min.max(0)
        })
    }

    fn calibration_host() -> Value {
        #[cfg(windows)]
        {
            serde_json::to_value(
                sky_player::adapter_support::build_host_fingerprint().expect("host fingerprint"),
            )
            .expect("host JSON")
        }
        #[cfg(not(windows))]
        serde_json::json!({
            "host_fingerprint_version": 2,
            "qpc_frequency_hz": 10_000_000,
            "win32_build": "Windows test",
            "processor_architecture": "AMD64",
            "cpu_vendor": "test",
            "cpu_family": 6,
            "cpu_model": 1,
            "cpu_stepping": 1,
            "logical_processor_count": 1,
            "processor_group_count": 1,
            "cpu_set_efficiency_classes": [0],
            "highest_efficiency_class": 0,
            "lowest_efficiency_class": 0,
            "sampled_at_us": 1
        })
    }

    fn valid_calibration_cache(sendinput_max: i64) -> Value {
        let mut pair_buckets = serde_json::Map::new();
        for key in CALIBRATION_REQUIRED_BUCKETS {
            pair_buckets.insert(
                key.into(),
                serde_json::json!({
                    "attempted": 100,
                    "clean_pair_count": 100,
                    "rejected": 0,
                    "anomaly_count": 0,
                    "class_mismatch_count": 0,
                    "timeout_count": 0,
                    "partial_send": 0,
                    "pair_sender_hold_shrink_us": quantiles(-4, sendinput_max),
                    "scheduler_shrink_us": quantiles(-4, 8),
                    "sendinput_shrink_us": quantiles(-4, sendinput_max),
                    "down_call_duration_us": quantiles(1, 3),
                    "up_call_duration_us": quantiles(1, 3)
                }),
            );
        }
        let status = if sendinput_max + 100 > 2_000 {
            "out_of_envelope"
        } else {
            "valid"
        };
        let applied =
            (300_i64.max(sendinput_max + 100) <= 2_000).then_some(300_i64.max(sendinput_max + 100));
        serde_json::json!({
            "version": 8,
            "evidence_kind": "sender_completion_hold_shrink",
            "artifact_schema_version": 11,
            "native_calibration_version": 15,
            "measurement_protocol_version": 10,
            "source": "device_cache",
            "source_formula_version": 6,
            "status": status,
            "source_git_sha": "test-sha",
            "native_build_id": "test-sha",
            "native_source_fingerprint": "test-fingerprint",
            "rustc_version": "rustc test",
            "dirty_worktree": false,
            "host_fingerprint": calibration_host(),
            "scheduling_aids": {
                "mmcss_acquired": "mmcss:Games",
                "mmcss_active": true,
                "power_throttling_active": true,
                "waiter_mode": "event+high_resolution_timer"
            },
            "required_buckets": CALIBRATION_REQUIRED_BUCKETS,
            "pair_buckets": pair_buckets,
            "transport_margin_us": applied,
            "transport_margin_source": "device_cache",
            "transport_worst_positive_us": sendinput_max,
            "transport_guard_us": 100,
            "transport_floor_us": 300,
            "transport_ceiling_us": 2000,
            "calibration_timing_qualified": status == "valid",
            "qualification": {
                "basis": "max_required_bucket_max_positive_sendinput_shrink",
                "worst_bucket": "1/hot",
                "transport_worst_positive_us": sendinput_max,
                "guard_us": 100,
                "floor_us": 300,
                "ceiling_us": 2000,
                "candidate_transport_margin_us": sendinput_max + 100,
                "applied_transport_margin_us": applied
            }
        })
    }

    #[test]
    fn catalog_source_rejects_non_directory_and_ignores_unsupported_files() {
        let root = std::env::temp_dir().join(format!("sky-w2-catalog-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp root");
        fs::write(root.join("song.txt"), "notes").expect("song");
        fs::write(root.join("ignored.csv"), "ignored").expect("ignored");
        let entries = FileCatalogSource::new(&root).entries().expect("entries");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].canonical_path.ends_with("song.txt"));
        let file = root.join("not-a-directory");
        fs::write(&file, "file").expect("file");
        assert!(matches!(
            FileCatalogSource::new(file).entries(),
            Err(CatalogError::SourceUnavailable(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn library_manifest_round_trips_and_imported_folders_are_composed() {
        let root = std::env::temp_dir().join(format!(
            "sky-library-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let songs = root.join("songs");
        let imported = root.join("imported/nested");
        fs::create_dir_all(&songs).expect("songs");
        fs::create_dir_all(&imported).expect("imported");
        fs::write(songs.join("primary.json"), "{}").expect("primary");
        fs::write(imported.join("local.txt"), "notes").expect("imported song");
        fs::write(imported.join("second.skysheet"), "notes").expect("second imported song");
        fs::write(imported.join("ignored.csv"), "ignored").expect("unsupported import");

        let canonical = fs::canonicalize(root.join("imported")).expect("canonical folder");
        let manifest = LibraryManifestV1 {
            version: LIBRARY_MANIFEST_VERSION,
            imports: vec![ImportedSourceRef {
                source_id: "a".repeat(32),
                canonical_path: canonical.to_string_lossy().into_owned(),
                kind: ImportedSourceKind::Folder,
            }],
            collections: Vec::new(),
        };
        let store = JsonLibraryManifestStore::new(root.join("library-manifest.json"));
        store.save(&manifest).expect("manifest save");
        assert_eq!(store.load().expect("manifest load"), manifest);

        let entries = FileCatalogSource::new(&songs)
            .entries_with_imports(&manifest.imports)
            .expect("composed entries");
        assert_eq!(entries.len(), 3);
        assert!(
            entries
                .iter()
                .any(|entry| entry.canonical_path.ends_with("local.txt"))
        );

        let composition = FileCatalogSource::new(&songs)
            .catalog_composition_with_imports(&[
                manifest.imports[0].clone(),
                ImportedSourceRef {
                    source_id: "b".repeat(32),
                    canonical_path: root.join("disconnected").to_string_lossy().into_owned(),
                    kind: ImportedSourceKind::Folder,
                },
            ])
            .expect("catalog composition");
        assert_eq!(composition.primary_membership.len(), 1);
        assert_eq!(composition.imported_status.len(), 2);
        assert_eq!(composition.imported_status[0].song_count, 2);
        assert!(composition.imported_status[0].available);
        assert_eq!(composition.imported_membership[&"a".repeat(32)].len(), 2);
        assert_eq!(composition.imported_status[1].song_count, 0);
        assert!(!composition.imported_status[1].available);
        assert!(composition.imported_membership[&"b".repeat(32)].is_empty());

        let duplicate = FileCatalogSource::new(&songs)
            .catalog_composition_with_imports(&[ImportedSourceRef {
                source_id: "c".repeat(32),
                canonical_path: songs.to_string_lossy().into_owned(),
                kind: ImportedSourceKind::Folder,
            }])
            .expect("duplicate composition");
        assert_eq!(duplicate.entries.len(), 1);
        assert_eq!(duplicate.imported_status[0].song_count, 1);
        assert_eq!(duplicate.imported_membership[&"c".repeat(32)].len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}

use super::composer::CatalogScanMetrics;
use sky_app_core::catalog::{CatalogError, CatalogSourceEntry, SUPPORTED_EXTENSIONS, SongSource};
use sky_app_core::library::ImportedSourceKind;
use std::fs;
use std::path::{Path, PathBuf};
pub struct FileCatalogSource {
    root: PathBuf,
}

/// Native-only catalog composition. The entries retain canonical paths for
/// local file access, while the membership/status projections are safe to
/// summarize across the IPC boundary.
impl FileCatalogSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entries_with_metrics(
        &self,
    ) -> Result<(Vec<CatalogSourceEntry>, CatalogScanMetrics), CatalogError> {
        entries_from_directory_with_metrics(&self.root, false)
    }
}

impl SongSource for FileCatalogSource {
    fn entries(&self) -> Result<Vec<CatalogSourceEntry>, CatalogError> {
        entries_from_directory(&self.root, false)
    }
}

pub(super) fn entries_from_file(path: &Path) -> Result<Vec<CatalogSourceEntry>, CatalogError> {
    if !path.is_file() || !is_supported(path) {
        return Ok(Vec::new());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
    Ok(vec![CatalogSourceEntry::path_derived(
        canonical.to_string_lossy(),
        path.file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
    )])
}

pub(super) fn entries_from_file_with_metrics(
    path: &Path,
) -> Result<(Vec<CatalogSourceEntry>, CatalogScanMetrics), CatalogError> {
    let mut metrics = CatalogScanMetrics::default();
    if path.exists() && path.is_file() {
        metrics.files_visited = 1;
        metrics.supported_files = u64::from(is_supported(path));
    }
    if !path.is_file() || !is_supported(path) {
        return Ok((Vec::new(), metrics));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
    Ok((
        vec![CatalogSourceEntry::path_derived(
            canonical.to_string_lossy(),
            path.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default(),
        )],
        metrics,
    ))
}

pub(super) fn entries_from_directory(
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

pub(super) fn entries_from_directory_with_metrics(
    root: &Path,
    recursive: bool,
) -> Result<(Vec<CatalogSourceEntry>, CatalogScanMetrics), CatalogError> {
    if !root.exists() {
        return Ok((Vec::new(), CatalogScanMetrics::default()));
    }
    if !root.is_dir() {
        return Err(CatalogError::SourceUnavailable(
            "songs directory is not a directory".into(),
        ));
    }
    let mut entries = Vec::new();
    let mut metrics = CatalogScanMetrics::default();
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        metrics.directories_visited = metrics.directories_visited.saturating_add(1);
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
                metrics.files_visited = metrics.files_visited.saturating_add(1);
                metrics.supported_files = metrics
                    .supported_files
                    .saturating_add(u64::from(is_supported(&path)));
                entries.extend(entries_from_file(&path)?);
            }
        }
        if !recursive {
            break;
        }
    }
    entries.sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
    Ok((entries, metrics))
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

pub(super) fn import_display_name(path: &Path, kind: ImportedSourceKind) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| match kind {
            ImportedSourceKind::File => "Imported file".to_owned(),
            ImportedSourceKind::Folder => "Imported folder".to_owned(),
        })
}

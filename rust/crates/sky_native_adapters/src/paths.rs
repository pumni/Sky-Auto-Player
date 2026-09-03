//! Application directory and path authority for the v4 desktop runtime.
//!
//! This module enforces the clean application-data boundary mandated by ADR-0006:
//! - The installer owns the complete installation root (`install_root`), which
//!   contains only immutable application payload (executables, DLLs, resources).
//! - The application owns its mutable state in OS-appropriate application-data
//!   locations (`config_root`, `data_root`, `cache_root`, `logs_root`).
//! - Music files belong to user-owned library sources (`user_music_root`), never
//!   requiring an updater preserve list inside the installation directory.
//! - Production `.env` files beside the executable are not part of the v4 configuration model.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// The permanent canonical v4 application identifier per ADR-0006.
pub const V4_APP_IDENTIFIER: &str = "io.github.pumni.skyautoplayer";

/// Name of the native calibration worker binary in the install payload.
pub const CALIBRATION_EXE: &str = "native_calibration.exe";

/// Default file name for application settings.
pub const CONFIG_FILE: &str = "config.json";

/// Default file name for the library manifest.
pub const LIBRARY_MANIFEST_FILE: &str = "library-manifest.json";

/// Default file name for the device calibration cache.
pub const CALIBRATION_CACHE_FILE: &str = "input_latency.json";

/// Default subdirectory name for cache files under the app-data root.
pub const CACHE_SUBDIR: &str = "cache";

/// Default subdirectory name for log files under the app-data root.
pub const LOGS_SUBDIR: &str = "logs";

/// Default subdirectory name for user music under the app-data root.
pub const DEFAULT_SONGS_SUBDIR: &str = "songs";

/// Canonical typed directory and path authority for Sky Auto Player v4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    install_root: PathBuf,
    config_root: PathBuf,
    data_root: PathBuf,
    cache_root: PathBuf,
    logs_root: PathBuf,
    user_music_root: PathBuf,
}

impl AppPaths {
    /// The canonical v4 application identifier.
    pub const APP_ID: &'static str = V4_APP_IDENTIFIER;

    /// Construct an explicit `AppPaths` instance from each typed root.
    pub fn from_roots(
        install_root: PathBuf,
        config_root: PathBuf,
        data_root: PathBuf,
        cache_root: PathBuf,
        logs_root: PathBuf,
        user_music_root: PathBuf,
    ) -> Self {
        Self {
            install_root,
            config_root,
            data_root,
            cache_root,
            logs_root,
            user_music_root,
        }
    }

    /// Construct an `AppPaths` instance from an install payload root and a base
    /// application-data directory.
    ///
    /// Per ADR-0006 target Windows layout:
    /// - `config_root`: `app_data_root`
    /// - `data_root`: `app_data_root`
    /// - `cache_root`: `app_data_root/cache` (deterministic canonical v4 cache)
    /// - `logs_root`: `app_data_root/logs`
    /// - `user_music_root`: `app_data_root/songs`
    pub fn from_app_data_root(install_root: PathBuf, app_data_root: PathBuf) -> Self {
        Self {
            cache_root: app_data_root.join(CACHE_SUBDIR),
            logs_root: app_data_root.join(LOGS_SUBDIR),
            user_music_root: app_data_root.join(DEFAULT_SONGS_SUBDIR),
            config_root: app_data_root.clone(),
            data_root: app_data_root,
            install_root,
        }
    }

    /// Convenience constructor for testing: places install payload under `sandbox/install`
    /// and all mutable application data under `sandbox/app_data`.
    pub fn from_test_sandbox(sandbox_root: &Path) -> Self {
        Self::from_app_data_root(sandbox_root.join("install"), sandbox_root.join("app_data"))
    }

    /// The installer-owned application root (immutable payload in v4).
    pub fn install_root(&self) -> &Path {
        &self.install_root
    }

    /// The application-owned mutable configuration directory (stores `config.json`).
    pub fn config_root(&self) -> &Path {
        &self.config_root
    }

    /// The application-owned mutable state/data directory (stores `library-manifest.json`).
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// The application-owned mutable cache directory.
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// The application-owned mutable logs directory.
    pub fn logs_root(&self) -> &Path {
        &self.logs_root
    }

    /// The base directory for user-owned music / song sheets outside the install root.
    pub fn user_music_root(&self) -> &Path {
        &self.user_music_root
    }

    /// Full path to the persisted `config.json` settings file.
    pub fn settings_path(&self) -> PathBuf {
        self.config_root.join(CONFIG_FILE)
    }

    /// Full path to the persisted `library-manifest.json` file.
    pub fn library_manifest_path(&self) -> PathBuf {
        self.data_root.join(LIBRARY_MANIFEST_FILE)
    }

    /// Full path to the calibration cache file (`input_latency.json`).
    ///
    /// Always resolves deterministically to `cache_root/input_latency.json`.
    pub fn calibration_cache_path(&self) -> PathBuf {
        self.cache_root.join(CALIBRATION_CACHE_FILE)
    }

    /// Full path to the installer-owned calibration executable.
    pub fn calibration_binary_path(&self) -> PathBuf {
        self.install_root.join(CALIBRATION_EXE)
    }

    /// Resolve a songs directory from user settings.
    ///
    /// Fails closed if:
    /// - `songs_dir` contains parent directory traversal components (`..`).
    /// - The resolved path resides inside `install_root`.
    ///
    /// Valid external absolute paths are preserved as-is.
    /// Default `"songs"` or empty string resolves to `user_music_root`.
    pub fn resolve_songs_dir(&self, songs_dir: &str) -> Result<PathBuf, String> {
        let path = Path::new(songs_dir);

        // Reject any traversal attempt using '..' components
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!(
                "v4 architecture violation: songs_dir ('{songs_dir}') must not contain parent directory traversal ('..')"
            ));
        }

        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else if songs_dir == DEFAULT_SONGS_SUBDIR || songs_dir.is_empty() {
            self.user_music_root.clone()
        } else {
            self.user_music_root.join(path)
        };

        // Reject if the resolved path resides inside the immutable install root
        if self.is_installer_owned(&resolved) {
            return Err(format!(
                "v4 architecture violation: resolved songs directory ('{}') must not reside inside install root ('{}')",
                resolved.display(),
                self.install_root.display()
            ));
        }

        Ok(resolved)
    }

    /// Check whether a path resides inside the installer-owned payload directory.
    pub fn is_installer_owned(&self, path: &Path) -> bool {
        path.starts_with(&self.install_root)
    }

    /// Check whether a path resides inside application-owned or user-owned mutable roots.
    pub fn is_app_data_owned(&self, path: &Path) -> bool {
        path.starts_with(&self.config_root)
            || path.starts_with(&self.data_root)
            || path.starts_with(&self.cache_root)
            || path.starts_with(&self.logs_root)
            || path.starts_with(&self.user_music_root)
    }

    /// Create all mutable app-data directories if they do not yet exist.
    ///
    /// Never touches or attempts to create directories inside `install_root`.
    pub fn ensure_mutable_directories(&self) -> Result<(), std::io::Error> {
        fs::create_dir_all(&self.config_root)?;
        fs::create_dir_all(&self.data_root)?;
        fs::create_dir_all(&self.cache_root)?;
        fs::create_dir_all(&self.logs_root)?;
        fs::create_dir_all(&self.user_music_root)?;
        Ok(())
    }

    /// Assert that all mutable data roots are strictly separated from `install_root`.
    ///
    /// Returns an error if any mutable root resides inside `install_root`.
    pub fn assert_clean_boundary(&self) -> Result<(), String> {
        let roots = [
            ("config_root", &self.config_root),
            ("data_root", &self.data_root),
            ("cache_root", &self.cache_root),
            ("logs_root", &self.logs_root),
            ("user_music_root", &self.user_music_root),
        ];
        for (name, path) in roots {
            if path.starts_with(&self.install_root) {
                return Err(format!(
                    "v4 architecture violation: {name} ({}) must not reside inside install root ({})",
                    path.display(),
                    self.install_root.display()
                ));
            }
        }
        Ok(())
    }

    /// Resolve canonical `AppPaths` for the running process from host environment.
    ///
    /// Fails closed if:
    /// - `SKY_INSTALL_ROOT` fails strict canonicalization.
    /// - The resolved mutable roots violate `assert_clean_boundary()`.
    pub fn resolve() -> Result<Self, String> {
        let install_root = resolve_install_root()?;
        let app_data_root = resolve_app_data_root()?;
        let user_music_root = resolve_user_music_root(&app_data_root);

        let paths = Self {
            config_root: app_data_root.clone(),
            data_root: app_data_root.clone(),
            cache_root: app_data_root.join(CACHE_SUBDIR),
            logs_root: app_data_root.join(LOGS_SUBDIR),
            user_music_root,
            install_root,
        };

        // Fail closed immediately if any mutable root is nested inside the install payload
        paths.assert_clean_boundary()?;

        Ok(paths)
    }
}

/// Snapshot all files under a directory: relative path -> (file size, sha256 hex).
///
/// Provides immutable proof that the installation directory was not touched or modified.
pub fn snapshot_directory(root: &Path) -> Result<BTreeMap<String, (u64, String)>, std::io::Error> {
    let mut files = BTreeMap::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_directory_files(root, root, &mut files)?;
    Ok(files)
}

fn collect_directory_files(
    base: &Path,
    current: &Path,
    files: &mut BTreeMap<String, (u64, String)>,
) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_directory_files(base, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(base)
                .map_err(std::io::Error::other)?
                .to_string_lossy()
                .replace('\\', "/");
            let data = fs::read(&path)?;
            let size = data.len() as u64;
            let digest = Sha256::digest(&data);
            let mut hash = String::with_capacity(64);
            for byte in digest {
                use std::fmt::Write as _;
                let _ = write!(hash, "{byte:02x}");
            }
            files.insert(relative, (size, hash));
        }
    }
    Ok(())
}

fn resolve_install_root() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("SKY_INSTALL_ROOT") {
        return fs::canonicalize(value)
            .map_err(|error| format!("invalid SKY_INSTALL_ROOT: {error}"));
    }
    if cfg!(debug_assertions) {
        let root = std::env::var_os("SKY_DESKTOP_REPOSITORY_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."));
        return fs::canonicalize(root)
            .map_err(|error| format!("invalid debug repository root: {error}"));
    }
    let executable =
        std::env::current_exe().map_err(|error| format!("cannot resolve executable: {error}"))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "executable has no install root".into())
}

fn resolve_app_data_root() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("SKY_APP_DATA_ROOT") {
        return Ok(PathBuf::from(value));
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local).join(V4_APP_IDENTIFIER));
        }
        if let Some(roaming) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(roaming).join(V4_APP_IDENTIFIER));
        }
        Err("neither LOCALAPPDATA nor APPDATA is available in environment".into())
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join(V4_APP_IDENTIFIER));
        }
        Err("HOME is unavailable in environment".into())
    }
}

fn resolve_user_music_root(app_data_root: &Path) -> PathBuf {
    if let Some(value) = std::env::var_os("SKY_SONGS_DIR") {
        return PathBuf::from(value);
    }
    app_data_root.join(DEFAULT_SONGS_SUBDIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_paths_clean_boundary_assertion_passes_when_separated() {
        let install_root = PathBuf::from(r"C:\Users\tester\AppData\Local\Programs\Sky Auto Player");
        let app_data =
            PathBuf::from(r"C:\Users\tester\AppData\Local\io.github.pumni.skyautoplayer");
        let paths = AppPaths::from_app_data_root(install_root.clone(), app_data.clone());

        assert_eq!(paths.install_root(), install_root.as_path());
        assert_eq!(paths.config_root(), app_data.as_path());
        assert_eq!(paths.data_root(), app_data.as_path());
        assert_eq!(paths.cache_root(), app_data.join("cache").as_path());
        assert_eq!(paths.logs_root(), app_data.join("logs").as_path());
        assert_eq!(paths.user_music_root(), app_data.join("songs").as_path());

        assert_eq!(paths.settings_path(), app_data.join("config.json"));
        assert_eq!(
            paths.library_manifest_path(),
            app_data.join("library-manifest.json")
        );
        assert_eq!(
            paths.calibration_cache_path(),
            app_data.join("cache").join("input_latency.json")
        );
        assert_eq!(
            paths.calibration_binary_path(),
            install_root.join("native_calibration.exe")
        );

        assert!(paths.assert_clean_boundary().is_ok());
    }

    #[test]
    fn app_paths_clean_boundary_assertion_fails_when_nested() {
        let install_root = PathBuf::from(r"C:\Program Files\Sky Auto Player");
        let nested = install_root.join("data");
        let paths = AppPaths::from_app_data_root(install_root, nested);

        let err = paths.assert_clean_boundary().unwrap_err();
        assert!(err.contains("v4 architecture violation"));
    }

    #[test]
    fn resolve_songs_dir_resolves_valid_relative_and_absolute_paths() {
        let install_root = PathBuf::from(r"C:\Program Files\Sky Auto Player");
        let app_data =
            PathBuf::from(r"C:\Users\tester\AppData\Local\io.github.pumni.skyautoplayer");
        let paths = AppPaths::from_app_data_root(install_root.clone(), app_data.clone());

        // Default relative "songs" resolves to user music root under app data
        let default_songs = paths.resolve_songs_dir("songs").expect("valid songs");
        assert_eq!(default_songs, app_data.join("songs"));
        assert!(!default_songs.starts_with(&install_root));

        // Subdirectory under user music
        let sub = paths
            .resolve_songs_dir("custom_sub")
            .expect("valid subpath");
        assert_eq!(sub, app_data.join("songs").join("custom_sub"));
        assert!(!sub.starts_with(&install_root));

        // Absolute external path is preserved
        let abs = paths
            .resolve_songs_dir(r"D:\MyMusic\SkySheets")
            .expect("valid external abs");
        assert_eq!(abs, PathBuf::from(r"D:\MyMusic\SkySheets"));
        assert!(!abs.starts_with(&install_root));
    }

    #[test]
    fn resolve_songs_dir_rejects_relative_parent_traversal() {
        let install_root = PathBuf::from(r"C:\Program Files\Sky Auto Player");
        let app_data =
            PathBuf::from(r"C:\Users\tester\AppData\Local\io.github.pumni.skyautoplayer");
        let paths = AppPaths::from_app_data_root(install_root, app_data);

        // Relative parent escape: "../escaped"
        let err = paths.resolve_songs_dir("../escaped").unwrap_err();
        assert!(err.contains("parent directory traversal ('..')"), "{err}");

        // Nested traversal: "songs/../../escaped"
        let err = paths.resolve_songs_dir("songs/../../escaped").unwrap_err();
        assert!(err.contains("parent directory traversal ('..')"), "{err}");
    }

    #[test]
    fn resolve_songs_dir_rejects_absolute_path_inside_install_root() {
        let install_root = PathBuf::from(r"C:\Program Files\Sky Auto Player");
        let app_data =
            PathBuf::from(r"C:\Users\tester\AppData\Local\io.github.pumni.skyautoplayer");
        let paths = AppPaths::from_app_data_root(install_root.clone(), app_data);

        // Absolute path targeting inside the installer-owned root
        let inside = install_root.join("songs");
        let err = paths
            .resolve_songs_dir(&inside.display().to_string())
            .unwrap_err();
        assert!(err.contains("must not reside inside install root"), "{err}");
    }

    #[test]
    fn snapshot_directory_detects_additions_modifications_and_deletions() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let temp = std::env::temp_dir().join(format!("sky-snapshot-test-{suffix}"));
        fs::create_dir_all(&temp).expect("create temp");

        let file_a = temp.join("a.txt");
        let sub = temp.join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        let file_b = sub.join("b.txt");

        fs::write(&file_a, b"content a").expect("write a");
        fs::write(&file_b, b"content b").expect("write b");

        let snap1 = snapshot_directory(&temp).expect("snapshot 1");
        assert_eq!(snap1.len(), 2);
        assert!(snap1.contains_key("a.txt"));
        assert!(snap1.contains_key("sub/b.txt"));

        // Exact equality when unchanged
        let snap2 = snapshot_directory(&temp).expect("snapshot 2");
        assert_eq!(snap1, snap2);

        // Modification is detected
        fs::write(&file_a, b"modified content a").expect("modify a");
        let snap_modified = snapshot_directory(&temp).expect("snapshot modified");
        assert_ne!(snap1, snap_modified);

        // Restore file_a
        fs::write(&file_a, b"content a").expect("restore a");

        // Addition is detected
        let file_c = temp.join("c.txt");
        fs::write(&file_c, b"content c").expect("write c");
        let snap_added = snapshot_directory(&temp).expect("snapshot added");
        assert_ne!(snap1, snap_added);
        assert_eq!(snap_added.len(), 3);

        // Deletion is detected
        fs::remove_file(&file_c).expect("remove c");
        fs::remove_file(&file_b).expect("remove b");
        let snap_deleted = snapshot_directory(&temp).expect("snapshot deleted");
        assert_ne!(snap1, snap_deleted);
        assert_eq!(snap_deleted.len(), 1);

        let _ = fs::remove_dir_all(&temp);
    }
}

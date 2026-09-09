use crate::Result;
use serde_json::to_vec_pretty;
use sha2::{Digest, Sha256};
use sky_app_core::catalog::{
    BuiltinCatalogManifest, BuiltinSongManifestEntry, RetiredBuiltinSong, SUPPORTED_EXTENSIONS,
    StableSongId,
};
use sky_app_core::song::parse_song_json;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MANIFEST_RELATIVE_PATH: &str = "builtin-songs/manifest.json";

pub fn run(root: &Path, operation: &str, args: &[(String, String)]) -> Result<()> {
    match operation {
        "verify" => verify(root),
        "verify-installed" => {
            let catalog_root = required(args, "root")?;
            verify_installed(Path::new(&catalog_root))
        }
        "refresh" => refresh(root),
        "add" => add(root, args),
        "rename" => rename(root, args),
        "retire" => retire(root, args),
        "restore" => restore(root, args),
        _ => Err(format!(
            "unknown builtin-catalog operation `{operation}`; expected verify, refresh, add, rename, retire, or restore"
        )
        .into()),
    }
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_RELATIVE_PATH)
}

fn load(root: &Path) -> Result<BuiltinCatalogManifest> {
    load_manifest(&manifest_path(root))
}

fn load_manifest(path: &Path) -> Result<BuiltinCatalogManifest> {
    let bytes = fs::read(path)?;
    let manifest: BuiltinCatalogManifest = serde_json::from_slice(&bytes)?;
    Ok(manifest)
}

fn source_path(root: &Path, manifest_path: &str) -> Result<PathBuf> {
    let relative = manifest_path.strip_prefix("sheets/").ok_or_else(|| {
        format!("built-in path is outside the repository source mapping: {manifest_path}")
    })?;
    if relative.is_empty() || relative.contains('/') || relative.contains('\\') {
        return Err(
            format!("built-in source path is not a top-level sheet: {manifest_path}").into(),
        );
    }
    Ok(root.join("songs").join(relative))
}

fn verify(root: &Path) -> Result<()> {
    let manifest = load(root)?;
    manifest
        .validate()
        .map_err(|error| format!("{MANIFEST_RELATIVE_PATH}: {error}"))?;

    let mut declared = BTreeSet::new();
    for song in &manifest.songs {
        let path = source_path(root, &song.path)?;
        if !path.is_file() {
            return Err(format!("declared built-in source is missing: {}", path.display()).into());
        }
        if !declared.insert(path.clone()) {
            return Err(format!(
                "built-in source is declared more than once: {}",
                path.display()
            )
            .into());
        }
        let digest = sha256_file(&path)?;
        if digest != song.sha256 {
            return Err(format!(
                "built-in hash mismatch for {}: manifest {}, actual {}",
                song.path, song.sha256, digest
            )
            .into());
        }
        let bytes = fs::read(&path)?;
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&song.title);
        parse_song_json(&bytes, fallback)
            .map_err(|error| format!("built-in song is not parseable ({}): {error}", song.path))?;
    }

    let mut discovered = BTreeSet::new();
    let songs_root = root.join("songs");
    if !songs_root.is_dir() {
        return Err(format!(
            "built-in source directory is missing: {}",
            songs_root.display()
        )
        .into());
    }
    for entry in WalkDir::new(&songs_root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() || !is_supported(entry.path()) {
            continue;
        }
        discovered.insert(entry.path().to_owned());
    }
    if discovered != declared {
        let missing = declared.difference(&discovered).collect::<Vec<_>>();
        let extra = discovered.difference(&declared).collect::<Vec<_>>();
        return Err(format!(
            "built-in manifest/source drift: missing={missing:?}, extra={extra:?}"
        )
        .into());
    }
    println!(
        "Built-in catalog verification: PASS (schema=1, active={}, retired={})",
        manifest.songs.len(),
        manifest.retired_songs.len()
    );
    Ok(())
}

fn verify_installed(root: &Path) -> Result<()> {
    let manifest = load_manifest(&root.join("manifest.json"))?;
    manifest
        .validate()
        .map_err(|error| format!("installed built-in manifest: {error}"))?;
    if !root.is_dir() {
        return Err(format!(
            "installed built-in catalog directory is missing: {}",
            root.display()
        )
        .into());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != "manifest.json" && name != "sheets" {
            return Err(format!(
                "installed built-in catalog contains an unexpected root entry: {}",
                entry.path().display()
            )
            .into());
        }
        if entry.file_type()?.is_symlink() {
            return Err(format!(
                "installed built-in catalog contains a symlink: {}",
                entry.path().display()
            )
            .into());
        }
    }

    let canonical_root = fs::canonicalize(root)?;
    let mut declared = BTreeSet::new();
    for song in &manifest.songs {
        let path = root.join(&song.path);
        let canonical_path = fs::canonicalize(&path).map_err(|error| {
            format!(
                "declared installed built-in resource is missing ({}): {error}",
                path.display()
            )
        })?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(format!(
                "installed built-in resource escapes its catalog root: {}",
                song.path
            )
            .into());
        }
        if !canonical_path.is_file() {
            return Err(format!(
                "declared installed built-in resource is not a file: {}",
                path.display()
            )
            .into());
        }
        let digest = sha256_file(&canonical_path)?;
        if digest != song.sha256 {
            return Err(format!(
                "installed built-in hash mismatch for {}: manifest {}, actual {}",
                song.path, song.sha256, digest
            )
            .into());
        }
        let bytes = fs::read(&canonical_path)?;
        let fallback = canonical_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&song.title);
        parse_song_json(&bytes, fallback).map_err(|error| {
            format!(
                "installed built-in song is not parseable ({}): {error}",
                song.path
            )
        })?;
        declared.insert(song.path.clone());
    }

    let sheets_root = root.join("sheets");
    if !sheets_root.is_dir() {
        return Err(format!(
            "installed built-in resource directory is missing: {}",
            sheets_root.display()
        )
        .into());
    }
    let mut discovered = BTreeSet::new();
    for entry in WalkDir::new(&sheets_root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "installed built-in resource tree contains a symlink: {}",
                entry.path().display()
            )
            .into());
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(format!(
                "installed built-in resource tree contains an unexpected entry: {}",
                entry.path().display()
            )
            .into());
        }
        if !is_supported(entry.path()) {
            return Err(format!(
                "installed built-in resource tree contains an unsupported file: {}",
                entry.path().display()
            )
            .into());
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| "installed built-in resource path escaped its catalog root")?
            .to_string_lossy()
            .replace('\\', "/");
        discovered.insert(relative);
    }
    if discovered != declared {
        let missing = declared.difference(&discovered).collect::<Vec<_>>();
        let extra = discovered.difference(&declared).collect::<Vec<_>>();
        return Err(format!(
            "installed built-in manifest/resource drift: missing={missing:?}, extra={extra:?}"
        )
        .into());
    }

    println!(
        "Installed built-in catalog verification: PASS (schema=1, active={}, retired={})",
        manifest.songs.len(),
        manifest.retired_songs.len()
    );
    Ok(())
}

fn refresh(root: &Path) -> Result<()> {
    let mut manifest = load(root)?;
    manifest
        .validate()
        .map_err(|error| format!("cannot refresh invalid manifest: {error}"))?;
    for song in &mut manifest.songs {
        let path = source_path(root, &song.path)?;
        song.sha256 = sha256_file(&path)?;
    }
    write_manifest(root, &manifest)?;
    verify(root)
}

fn add(root: &Path, args: &[(String, String)]) -> Result<()> {
    let path = required(args, "path")?;
    let source = source_path(root, &path)?;
    if !source.is_file() {
        return Err(format!("built-in source is missing: {}", source.display()).into());
    }
    let mut manifest = load(root)?;
    let id = match args.iter().find(|(key, _)| key == "id") {
        Some((_, value)) => StableSongId::new(value.clone())?,
        None => StableSongId::new(deterministic_id(&path))?,
    };
    if manifest
        .songs
        .iter()
        .any(|song| song.path == path || song.id == id.as_str())
        || manifest
            .retired_songs
            .iter()
            .any(|song| song.id == id.as_str())
    {
        return Err("built-in path or identity already exists".into());
    }
    let title = args
        .iter()
        .find(|(key, _)| key == "title")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| {
            source
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("Built-in song")
                .to_owned()
        });
    manifest.songs.push(BuiltinSongManifestEntry {
        id: id.as_str().to_owned(),
        path,
        title,
        sha256: sha256_file(&source)?,
    });
    manifest
        .songs
        .sort_by(|left, right| left.path.cmp(&right.path));
    write_manifest(root, &manifest)?;
    verify(root)
}

fn rename(root: &Path, args: &[(String, String)]) -> Result<()> {
    let from = required(args, "from")?;
    let to = required(args, "to")?;
    let source = source_path(root, &from)?;
    let target = source_path(root, &to)?;
    if !source.is_file() || target.exists() {
        return Err("rename requires an existing source and a new target path".into());
    }
    let mut manifest = load(root)?;
    let item = manifest
        .songs
        .iter_mut()
        .find(|song| song.path == from)
        .ok_or("built-in rename source is not active")?;
    fs::rename(&source, &target)?;
    item.path = to;
    item.sha256 = sha256_file(&target)?;
    write_manifest(root, &manifest)?;
    verify(root)
}

fn retire(root: &Path, args: &[(String, String)]) -> Result<()> {
    let id = required(args, "id")?;
    let mut manifest = load(root)?;
    let index = manifest
        .songs
        .iter()
        .position(|song| song.id == id)
        .ok_or("built-in retirement identity is not active")?;
    let item = manifest.songs.remove(index);
    let source = source_path(root, &item.path)?;
    fs::remove_file(source)?;
    manifest.retired_songs.push(RetiredBuiltinSong {
        id: item.id,
        last_title: item.title,
    });
    write_manifest(root, &manifest)?;
    verify(root)
}

fn restore(root: &Path, args: &[(String, String)]) -> Result<()> {
    let id = required(args, "id")?;
    let path = required(args, "path")?;
    let mut manifest = load(root)?;
    let index = manifest
        .retired_songs
        .iter()
        .position(|song| song.id == id)
        .ok_or("built-in restore identity is not retired")?;
    let retired = manifest.retired_songs.remove(index);
    let source = source_path(root, &path)?;
    if !source.is_file() {
        return Err(format!("restored built-in source is missing: {}", source.display()).into());
    }
    let title = args
        .iter()
        .find(|(key, _)| key == "title")
        .map(|(_, value)| value.clone())
        .unwrap_or(retired.last_title);
    manifest.songs.push(BuiltinSongManifestEntry {
        id,
        path,
        title,
        sha256: sha256_file(&source)?,
    });
    manifest
        .songs
        .sort_by(|left, right| left.path.cmp(&right.path));
    write_manifest(root, &manifest)?;
    verify(root)
}

fn write_manifest(root: &Path, manifest: &BuiltinCatalogManifest) -> Result<()> {
    manifest
        .validate()
        .map_err(|error| format!("refusing to write invalid built-in manifest: {error}"))?;
    fs::write(manifest_path(root), to_vec_pretty(manifest)?)?;
    Ok(())
}

fn required(args: &[(String, String)], key: &str) -> Result<String> {
    args.iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| format!("builtin-catalog operation requires --{key} <value>").into())
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

fn sha256_file(path: &Path) -> Result<String> {
    let digest = Sha256::digest(fs::read(path)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn deterministic_id(path: &str) -> String {
    let digest = Sha256::digest(format!("sky-v4-builtin:{path}").as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

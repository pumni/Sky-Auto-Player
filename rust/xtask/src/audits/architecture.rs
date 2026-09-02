use crate::Result;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const FORBIDDEN_APP_CORE_DEPENDENCIES: &[&str] = &[
    "tauri",
    "pyo3",
    "windows-sys",
    "sky_desktop_shell",
    "sky_player",
    "sky_native_adapters",
];

fn app_core_boundary(root: &Path) -> Result<()> {
    let manifest_path = root.join("rust/crates/sky_app_core/Cargo.toml");
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(&manifest_path)?)?;
    if let Some(dependencies) = manifest.get("dependencies").and_then(toml::Value::as_table)
        && let Some(dependency) = FORBIDDEN_APP_CORE_DEPENDENCIES
            .iter()
            .find(|name| dependencies.contains_key(**name))
    {
        return Err(format!("sky_app_core must not depend directly on {dependency}").into());
    }
    let source_root = root.join("rust/crates/sky_app_core/src");
    for entry in WalkDir::new(source_root)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|ext| ext.to_str()) != Some("rs")
        {
            continue;
        }
        let source = fs::read_to_string(entry.path())?;
        let clean = crate::checks::strip_rust_comments(&source);
        let tokens = [
            "tauri",
            "pyo3",
            "windows-sys",
            "windows_sys",
            "sky_desktop_shell",
            "sky_player",
            "sky_native_adapters",
            "std::fs",
            "std::net",
        ];
        if let Some(token) = tokens.iter().find(|token| clean.contains(**token)) {
            return Err(format!(
                "sky_app_core source contains forbidden boundary reference `{token}` in {}",
                entry.path().display()
            )
            .into());
        }
        let without_forbid = clean.replace("#![forbid(unsafe_code)]", "");
        if without_forbid
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|word| word == "unsafe")
        {
            return Err(format!(
                "sky_app_core contains unsafe code in {}",
                entry.path().display()
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    app_core_boundary(root)?;
    crate::checks::architecture(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_core_boundary_passes() {
        app_core_boundary(&crate::repo::root()).expect("sky_app_core boundary");
    }
}

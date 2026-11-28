use crate::Result;
use std::path::Path;
use toml::Value;

const DEPENDENCY_KINDS: &[&str] = &["dependencies", "build-dependencies", "dev-dependencies"];
pub(super) const FORBIDDEN_APP_CORE_DEPENDENCIES: &[&str] = &[
    "tauri",
    "windows-sys",
    "sky_desktop_shell",
    "sky_player",
    "sky_native_adapters",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DependencyEdge {
    pub(super) key: String,
    pub(super) package: String,
    pub(super) kind: String,
    pub(super) target: Option<String>,
}

fn dependency_package_name(key: &str, declaration: &Value, owner: &str) -> Result<String> {
    match declaration {
        Value::String(_) => Ok(key.to_owned()),
        Value::Table(table) => match table.get("package") {
            None => Ok(key.to_owned()),
            Some(Value::String(package))
                if !package.trim().is_empty() && package.trim() == package =>
            {
                Ok(package.clone())
            }
            Some(_) => {
                Err(format!("{owner} dependency `{key}` has a malformed package identity").into())
            }
        },
        _ => Err(format!("{owner} dependency `{key}` must be a string or table").into()),
    }
}

fn inherited_dependency_package_name(
    key: &str,
    declaration: &Value,
    workspace: &Value,
    owner: &str,
) -> Result<String> {
    let Some(declaration_table) = declaration.as_table() else {
        return dependency_package_name(key, declaration, owner);
    };
    match declaration_table.get("workspace") {
        Some(Value::Boolean(true)) => {}
        Some(_) => {
            return Err(format!(
                "{owner} dependency `{key}` has a malformed workspace inheritance marker"
            )
            .into());
        }
        None => return dependency_package_name(key, declaration, owner),
    }
    if declaration_table.contains_key("package") {
        return Err(format!(
            "{owner} inherited dependency `{key}` must take its package identity from the workspace declaration"
        )
        .into());
    }
    if declaration_table.get("features").is_some_and(|features| {
        features
            .as_array()
            .is_none_or(|features| features.iter().any(|feature| feature.as_str().is_none()))
    }) {
        return Err(format!("{owner} inherited dependency `{key}` has malformed features").into());
    }
    for flag in ["optional", "default-features"] {
        if declaration_table
            .get(flag)
            .is_some_and(|value| value.as_bool().is_none())
        {
            return Err(
                format!("{owner} inherited dependency `{key}` has malformed `{flag}`").into(),
            );
        }
    }

    let workspace_dependencies = workspace
        .get("workspace")
        .and_then(Value::as_table)
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(Value::as_table)
        .ok_or_else(|| {
            format!(
                "{owner} inherits dependency `{key}`, but the root workspace has no valid [workspace.dependencies] table"
            )
        })?;
    let workspace_declaration = workspace_dependencies.get(key).ok_or_else(|| {
        format!(
            "{owner} inherits dependency `{key}`, which is missing from [workspace.dependencies]"
        )
    })?;
    if workspace_declaration
        .as_table()
        .is_some_and(|table| table.contains_key("workspace"))
    {
        return Err(format!(
            "root workspace dependency `{key}` cannot inherit another workspace declaration"
        )
        .into());
    }
    dependency_package_name(key, workspace_declaration, "root workspace")
}

fn append_dependency_table(
    result: &mut Vec<DependencyEdge>,
    manifest: &Value,
    workspace: &Value,
    owner: &str,
    kind: &str,
    target: Option<&str>,
) -> Result<()> {
    let Some(value) = manifest.get(kind) else {
        return Ok(());
    };
    let table = value
        .as_table()
        .ok_or_else(|| format!("{owner} [{kind}] must be a table"))?;
    for (key, declaration) in table {
        result.push(DependencyEdge {
            key: key.clone(),
            package: inherited_dependency_package_name(key, declaration, workspace, owner)?,
            kind: kind.to_owned(),
            target: target.map(str::to_owned),
        });
    }
    Ok(())
}

pub(super) fn resolve_manifest_dependencies(
    manifest: &Value,
    workspace: &Value,
    owner: &str,
) -> Result<Vec<DependencyEdge>> {
    let mut dependencies = Vec::new();
    for kind in DEPENDENCY_KINDS {
        append_dependency_table(&mut dependencies, manifest, workspace, owner, kind, None)?;
    }

    if let Some(targets) = manifest.get("target") {
        let targets = targets
            .as_table()
            .ok_or_else(|| format!("{owner} [target] must be a table"))?;
        for (target, target_manifest) in targets {
            if !target_manifest.is_table() {
                return Err(format!("{owner} target `{target}` must be a table").into());
            }
            for kind in DEPENDENCY_KINDS {
                append_dependency_table(
                    &mut dependencies,
                    target_manifest,
                    workspace,
                    owner,
                    kind,
                    Some(target),
                )?;
            }
        }
    }
    Ok(dependencies)
}

pub(super) fn app_core_dependency_boundary(manifest: &Value, workspace: &Value) -> Result<()> {
    for dependency in resolve_manifest_dependencies(manifest, workspace, "sky_app_core")? {
        if FORBIDDEN_APP_CORE_DEPENDENCIES.contains(&dependency.package.as_str()) {
            return Err(format!(
                "sky_app_core must not depend on {} (dependency key `{}` in {}{})",
                dependency.package,
                dependency.key,
                dependency.kind,
                dependency
                    .target
                    .map(|target| format!(" for `{target}`"))
                    .unwrap_or_default()
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn adapter_dependency_boundary(manifest: &Value, workspace: &Value) -> Result<()> {
    let dependencies = resolve_manifest_dependencies(manifest, workspace, "sky_native_adapters")?;
    let mut has_windows_dispatch_dependency = false;
    for dependency in dependencies {
        match dependency.package.as_str() {
            "sky_player" => {
                return Err("sky_native_adapters must not depend on sky_player".into());
            }
            "sky_dispatch_win32" => {
                if dependency.kind != "dependencies"
                    || dependency.target.as_deref() != Some("cfg(windows)")
                {
                    return Err(
                        "sky_native_adapters may depend on sky_dispatch_win32 only as a Windows normal dependency"
                            .into(),
                    );
                }
                has_windows_dispatch_dependency = true;
            }
            _ => {}
        }
    }
    if !has_windows_dispatch_dependency {
        return Err(
            "sky_native_adapters must declare its sky_dispatch_win32 dependency for Windows".into(),
        );
    }
    Ok(())
}

pub(super) fn manifest_at(root: &Path, relative: &str) -> Result<Value> {
    let path = root.join(relative);
    toml::from_str(&std::fs::read_to_string(path)?).map_err(Into::into)
}

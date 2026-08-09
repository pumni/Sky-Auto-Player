use std::fs;
use std::path::Path;

use crate::archive::validate_zip_file;
use crate::error::{Result, UpdaterError};
use crate::manifest::Manifest;
use crate::transaction::{TransactionPlan, apply, build_plan, prepare_journal, safe_join};
use crate::{MANIFEST_NAME, PRIMARY_EXE, UPDATER_EXE};

pub fn read_staged_manifest(staging: &Path, target_version: &str) -> Result<Manifest> {
    let manifest_path = safe_join(staging, MANIFEST_NAME)?;
    let manifest = Manifest::parse(&fs::read(manifest_path)?)?;
    manifest.validate(Some(target_version))?;
    manifest.verify_staged(staging)?;
    for required in [PRIMARY_EXE, UPDATER_EXE, "native_calibration.exe"] {
        if !staging.join(required).is_file() {
            return Err(UpdaterError::ManifestInvalid(format!(
                "required payload missing: {required}"
            )));
        }
    }
    Ok(manifest)
}

pub fn inspect_archive(path: &Path) -> Result<()> {
    validate_zip_file(path)?;
    Ok(())
}

pub fn installed_manifest(root: &Path) -> Result<Manifest> {
    let manifest_path = safe_join(root, MANIFEST_NAME).map_err(|err| {
        UpdaterError::ManifestInvalid(format!("installed manifest path is unsafe: {err}"))
    })?;
    Manifest::parse(&fs::read(manifest_path)?).map_err(|err| {
        UpdaterError::ManifestInvalid(format!("installed manifest unavailable: {err}"))
    })
}

pub fn install_verified(
    install_root: &Path,
    staging: &Path,
    new_manifest: &Manifest,
    old_manifest: &Manifest,
) -> Result<TransactionPlan> {
    let plan = build_plan(Some(old_manifest), new_manifest)?;
    prepare_journal(install_root, &plan)?;
    apply(install_root, staging, new_manifest, &plan)?;
    Ok(plan)
}

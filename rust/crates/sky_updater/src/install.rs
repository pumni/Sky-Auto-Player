use std::fs;
use std::path::Path;

use crate::archive::validate_zip_file;
use crate::error::{Result, UpdaterError, io_context};
use crate::manifest::Manifest;
use crate::transaction::{
    TransactionPlan, TransactionReport, apply, build_plan, preflight, prepare_journal, safe_join,
};
use crate::{MANIFEST_NAME, PRIMARY_EXE, UPDATER_EXE};

pub fn read_staged_manifest(staging: &Path, target_version: &str) -> Result<Manifest> {
    let manifest_path = safe_join(staging, MANIFEST_NAME)?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        io_context(
            "verify staging",
            "read staged manifest",
            &manifest_path,
            error,
        )
    })?;
    let manifest = Manifest::parse(&manifest_bytes)?;
    manifest.validate(Some(target_version))?;
    manifest.verify_staged(staging)?;
    for required in [PRIMARY_EXE, UPDATER_EXE, crate::CALIBRATION_EXE] {
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
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        io_context(
            "preflight",
            "read installed manifest",
            &manifest_path,
            error,
        )
    })?;
    Manifest::parse(&manifest_bytes).map_err(|err| {
        UpdaterError::ManifestInvalid(format!("installed manifest unavailable: {err}"))
    })
}

pub fn install_verified(
    install_root: &Path,
    staging: &Path,
    new_manifest: &Manifest,
    old_manifest: &Manifest,
) -> Result<InstallReport> {
    let plan = build_plan(Some(old_manifest), new_manifest)?;
    preflight(install_root, &plan)?;
    prepare_journal(install_root, &plan)?;
    let transaction = apply(install_root, staging, new_manifest, &plan)?;
    Ok(InstallReport { plan, transaction })
}

#[derive(Debug)]
pub struct InstallReport {
    pub plan: TransactionPlan,
    pub transaction: TransactionReport,
}

use super::*;

#[test]
fn website_validation_contract_requires_direct_checks_and_rejects_removed_guards() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let action = fs::read_to_string(root.join(".github/actions/site-validate/action.yml"))
        .expect("website action fixture must exist");
    let package = fs::read_to_string(root.join("apps/website/package.json"))
        .expect("website package fixture must exist");
    website_validation_contract(&action, &package)
        .expect("the direct website validation pipeline must satisfy its contract");

    let missing_seo_guard = action.replace("run: bun run verify:seo", "run: bun run verify:dist");
    assert_ne!(
        missing_seo_guard, action,
        "SEO guard fixture mutation must apply"
    );
    assert!(
        website_validation_contract(&missing_seo_guard, &package).is_err(),
        "the website action contract must fail when its required SEO guard is removed"
    );

    let unvalidated_metadata = action.replace("run: bun run test:unit", "run: bun run check");
    assert_ne!(unvalidated_metadata, action);
    assert!(website_validation_contract(&unvalidated_metadata, &package).is_err());

    let wrong_build = action.replace("run: bun run build", "run: bun run build:ci");
    assert_ne!(wrong_build, action);
    assert!(website_validation_contract(&wrong_build, &package).is_err());

    let missing_build = action.replace("run: bun run build", "run: bun run verify:dist");
    assert_ne!(missing_build, action);
    assert!(website_validation_contract(&missing_build, &package).is_err());

    let disabled_check = action.replace(
        "      run: bun run check",
        "      if: false\n      run: bun run check",
    );
    assert_ne!(disabled_check, action);
    assert!(website_validation_contract(&disabled_check, &package).is_err());

    let resurrected_hook = action.replace(
        "      run: bun run test:unit",
        "      run: cargo xtask version sync",
    );
    assert_ne!(resurrected_hook, action);
    assert!(website_validation_contract(&resurrected_hook, &package).is_err());

    let package_with_hook = package.replace(
        "  \"scripts\": {",
        "  \"scripts\": {\n    \"prebuild\": \"cargo xtask version sync\",",
    );
    assert_ne!(package_with_hook, package);
    assert!(website_validation_contract(&action, &package_with_hook).is_err());
}

#[test]
fn website_artifact_and_required_status_contracts_fail_closed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("CI workflow fixture must exist");
    let site_start = ci.find("\n  site:\n").expect("site job must exist");
    let status_start = ci.find("\n  status:\n").expect("status gate must exist");
    let site = &ci[site_start..status_start];
    website_artifact_upload_contract(site, "CI fixture")
        .expect("CI must upload one correctly placed website artifact");

    let misplaced_path = site.replace("path: apps/website/dist", "path: site/dist");
    assert_ne!(misplaced_path, site);
    assert!(website_artifact_upload_contract(&misplaced_path, "CI fixture").is_err());

    let missing_upload = site.replace(
        "actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9",
        "actions/upload-artifact@fixture",
    );
    assert_ne!(missing_upload, site);
    assert!(website_artifact_upload_contract(&missing_upload, "CI fixture").is_err());

    let duplicated_upload = site.replace(
        "actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9",
        "actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9\n        # actions/upload-pages-artifact@duplicate",
    );
    assert_ne!(duplicated_upload, site);
    assert!(website_artifact_upload_contract(&duplicated_upload, "CI fixture").is_err());

    let misplaced_upload = "path: apps/website/dist\nuses: actions/upload-pages-artifact@fixture";
    assert!(website_artifact_upload_contract(misplaced_upload, "fixture").is_err());

    let status = &ci[status_start..ci.find("\n  deploy_pages:\n").unwrap()];
    website_required_status_contract(status).expect("website result must gate required CI status");
    let disabled_gate = status.replace("[[ \"$SITE_RESULT\" == \"success\" ]]", "echo skipped");
    assert_ne!(disabled_gate, status);
    assert!(website_required_status_contract(&disabled_gate).is_err());
}

#[test]
fn pages_recovery_contract_rejects_missing_seo_duplicate_build_and_bad_artifact() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pages = fs::read_to_string(root.join(".github/workflows/pages.yml"))
        .expect("Pages recovery workflow fixture must exist");
    website_recovery_workflow_contract(&pages)
        .expect("manual Pages recovery must deploy the validated built artifact");

    let missing_seo = pages.replace("run: bun run verify:seo", "run: bun run verify:dist");
    assert_ne!(missing_seo, pages);
    assert!(website_recovery_workflow_contract(&missing_seo).is_err());

    let duplicate_build = pages.replace("  deploy:\n", "  deploy:\n    run: bun run build\n");
    assert_ne!(duplicate_build, pages);
    assert!(website_recovery_workflow_contract(&duplicate_build).is_err());

    let bad_artifact = pages.replace("path: apps/website/dist", "path: apps/desktop/dist");
    assert_ne!(bad_artifact, pages);
    assert!(website_recovery_workflow_contract(&bad_artifact).is_err());

    let missing_gate = pages.replace("needs: build", "needs: []");
    assert_ne!(missing_gate, pages);
    assert!(website_recovery_workflow_contract(&missing_gate).is_err());
}

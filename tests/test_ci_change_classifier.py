from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).parents[1]


def _load():
    path = ROOT / "scripts" / "classify_ci_changes.py"
    spec = importlib.util.spec_from_file_location("classify_ci_changes", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load CI classifier")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_docs_and_site_changes_skip_code_and_package_layers() -> None:
    result = _load().classify(
        ["README.md", "docs/evidence/desktop-phase9/README.md", "site/src/pages/index.astro"]
    )
    assert result.static_required
    assert not result.code_required
    assert not result.package_required


def test_regression_test_changes_run_code_but_not_package_build() -> None:
    result = _load().classify(["tests/test_phase9_gui_canonical.py"])
    assert result.static_required
    assert result.code_required
    assert not result.package_required


def test_runtime_and_release_inputs_require_portable_qualification() -> None:
    result = _load().classify(
        ["src/sky_music/cli/desktop_core.py", "desktop/src-tauri/src/main.rs"]
    )
    assert result.static_required
    assert result.code_required
    assert result.package_required


def test_ci_and_release_workflow_changes_require_portable_qualification() -> None:
    audit = _load()
    assert audit.classify([".github/workflows/ci.yml"]).package_required
    assert audit.classify([".github/actions/python-environment/action.yml"]).package_required
    assert audit.classify(["windows_version_info.txt"]).package_required
    assert not audit.classify([".github/workflows/site-ci.yml"]).package_required


def test_full_validation_overrides_path_classification() -> None:
    audit = _load()
    result = audit.classify(["README.md"], force_full=True)
    assert result == audit.ChangeClasses(True, True, True, "full validation requested")

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from typing import Any

import pytest

ROOT = Path(__file__).parents[1]


def _common() -> Any:
    path = ROOT / "scripts" / "release_common.py"
    spec = importlib.util.spec_from_file_location("wave5_release_common", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load release_common")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _builder() -> Any:
    path = ROOT / "scripts" / "build_portable_release.py"
    scripts = str(ROOT / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    spec = importlib.util.spec_from_file_location("wave5_portable_builder", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load portable builder")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _metadata(commit: str) -> tuple[dict[str, Any], dict[str, Any]]:
    desktop = {
        "schema_version": 1,
        "native_build_commit": commit,
        "native_version": "3.5.0",
        "rustc_version": "rustc 1.98.0",
        "win32_backend": True,
    }
    calibration = {
        "calibration_schema_version": 4,
        "measurement_protocol_version": 4,
        "evidence_kind": "paired_sender_timing",
        "host_fingerprint_version": 2,
        "source_git_sha": commit,
        "native_build_id": commit,
        "native_source_fingerprint": "f" * 64,
        "dirty_worktree": False,
    }
    return desktop, calibration


def test_matching_observed_desktop_and_calibration_metadata_is_accepted() -> None:
    common = _common()
    commit = "a" * 40
    desktop, calibration = _metadata(commit)

    assert (
        common.validate_observed_native_metadata(
            repo_head=commit,
            version="3.5.0",
            source_fingerprint="f" * 64,
            desktop_metadata=desktop,
            calibration_metadata=calibration,
        )
        == commit
    )


def test_builder_observes_and_qualifies_exact_binary_metadata(monkeypatch: pytest.MonkeyPatch) -> None:
    builder = _builder()
    commit = "a" * 40
    desktop, calibration = _metadata(commit)
    seen: list[tuple[list[str], str]] = []

    def capture(command: list[str], *, label: str) -> dict[str, Any]:
        seen.append((command, label))
        return desktop if label == "desktop" else calibration

    monkeypatch.setattr(builder, "_capture_native_metadata", capture)
    assert (
        builder.observe_native_build_metadata(
            Path("built-desktop.exe"),
            Path("built-calibration.exe"),
            repo_head=commit,
            version="3.5.0",
            source_fingerprint="f" * 64,
        )
        == commit
    )
    assert seen == [
        (["built-desktop.exe", "--selftest-build-info"], "desktop"),
        (["built-calibration.exe", "--metadata"], "calibration"),
    ]


def test_builder_rejects_observed_desktop_mismatch(monkeypatch: pytest.MonkeyPatch) -> None:
    builder = _builder()
    desktop, calibration = _metadata("b" * 40)
    monkeypatch.setattr(
        builder,
        "_capture_native_metadata",
        lambda _command, *, label: desktop if label == "desktop" else calibration,
    )
    with pytest.raises(RuntimeError, match="does not match repository HEAD"):
        builder.observe_native_build_metadata(
            Path("built-desktop.exe"),
            Path("built-calibration.exe"),
            repo_head="a" * 40,
            version="3.5.0",
            source_fingerprint="f" * 64,
        )


def test_desktop_metadata_mismatch_fails_qualification() -> None:
    common = _common()
    desktop, calibration = _metadata("b" * 40)
    with pytest.raises(RuntimeError, match="does not match repository HEAD"):
        common.validate_observed_native_metadata(
            repo_head="a" * 40,
            version="3.5.0",
            source_fingerprint="f" * 64,
            desktop_metadata=desktop,
            calibration_metadata=calibration,
        )


def test_calibration_metadata_mismatch_fails_qualification() -> None:
    common = _common()
    desktop, calibration = _metadata("a" * 40)
    calibration["source_git_sha"] = "b" * 40
    with pytest.raises(RuntimeError, match="does not match repository HEAD"):
        common.validate_observed_native_metadata(
            repo_head="a" * 40,
            version="3.5.0",
            source_fingerprint="f" * 64,
            desktop_metadata=desktop,
            calibration_metadata=calibration,
        )


@pytest.mark.parametrize(
    "output,label",
    [("not-json", "desktop"), ("[]", "calibration")],
)
def test_missing_or_malformed_native_metadata_is_rejected(output: str, label: str) -> None:
    with pytest.raises(RuntimeError):
        _common().parse_native_metadata(output, label=label)


def test_metadata_object_without_required_fields_is_rejected_by_qualification() -> None:
    common = _common()
    desktop, calibration = _metadata("a" * 40)
    desktop.clear()
    with pytest.raises(RuntimeError, match="omitted native_build_commit"):
        common.validate_observed_native_metadata(
            repo_head="a" * 40,
            version="3.5.0",
            source_fingerprint="f" * 64,
            desktop_metadata=desktop,
            calibration_metadata=calibration,
        )


def test_unknown_or_missing_required_metadata_is_rejected() -> None:
    common = _common()
    desktop, calibration = _metadata("a" * 40)
    del desktop["native_build_commit"]
    with pytest.raises(RuntimeError, match="omitted native_build_commit"):
        common.validate_observed_native_metadata(
            repo_head="a" * 40,
            version="3.5.0",
            source_fingerprint="f" * 64,
            desktop_metadata=desktop,
            calibration_metadata=calibration,
        )


def test_release_builder_uses_observed_commit_not_repo_assignment() -> None:
    source = (ROOT / "scripts" / "build_portable_release.py").read_text(encoding="utf-8")
    assert "built_native_commit = observe_native_build_metadata" in source
    assert "copied_native_commit = observe_native_build_metadata" in source
    assert "native_build_commit=repo_head" not in source


def test_native_source_fingerprint_ignores_generated_tauri_bindings(tmp_path: Path) -> None:
    common = _common()
    before = common.native_source_fingerprint(tmp_path)
    generated = tmp_path / "desktop" / "src-tauri" / "gen" / "schemas"
    generated.mkdir(parents=True)
    (generated / "bindings.json").write_text("generated", encoding="utf-8")
    assert common.native_source_fingerprint(tmp_path) == before

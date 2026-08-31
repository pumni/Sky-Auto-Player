from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import ModuleType

import pytest


def _load_build_module() -> ModuleType:
    path = Path(__file__).parents[1] / "scripts" / "build_rust_wheel.py"
    spec = importlib.util.spec_from_file_location("build_rust_wheel_under_test", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


BUILD = _load_build_module()


def test_wheel_build_reads_exact_toolchain_and_checks_compiler_metadata() -> None:
    rust_dir = Path(__file__).parents[1] / "rust"
    assert BUILD.pinned_rust_toolchain(rust_dir) == "1.98.0"
    BUILD.verify_build_info(
        {
            "rustc_version": "rustc 1.98.0 (test)",
            "native_abi": "cp314t-win_amd64",
            "native_build_commit": "commit",
        },
        expected_commit="commit",
        expected_rustc_prefix="rustc 1.98.0 ",
    )
    with pytest.raises(RuntimeError, match="wrong compiler"):
        BUILD.verify_build_info(
            {
                "rustc_version": "rustc 1.97.1 (wrong-toolchain)",
                "native_abi": "cp314t-win_amd64",
                "native_build_commit": "commit",
            },
            expected_commit="commit",
            expected_rustc_prefix="rustc 1.98.0 ",
        )


def test_cargo_profile_arguments_are_explicit_and_bounded() -> None:
    assert BUILD.cargo_profile_arguments("release") == ["--profile", "release"]
    assert BUILD.cargo_profile_arguments("dist") == ["--profile", "dist"]
    with pytest.raises(ValueError, match="unsupported Cargo profile"):
        BUILD.cargo_profile_arguments("debug")


def test_wheel_profile_defaults_fail_safe_by_build_kind() -> None:
    assert BUILD.resolve_cargo_profile(None, test_support=False) == "dist"
    assert BUILD.resolve_cargo_profile(None, test_support=True) == "release"


def test_wheel_profile_explicit_values_are_honored_for_both_build_kinds() -> None:
    assert BUILD.resolve_cargo_profile("dist", test_support=False) == "dist"
    assert BUILD.resolve_cargo_profile("dist", test_support=True) == "dist"
    assert BUILD.resolve_cargo_profile("release", test_support=False) == "release"
    assert BUILD.resolve_cargo_profile("release", test_support=True) == "release"


def test_wheel_profile_resolution_rejects_unsupported_profiles() -> None:
    with pytest.raises(ValueError, match="unsupported Cargo profile"):
        BUILD.resolve_cargo_profile("debug", test_support=False)


def test_build_commit_requires_clean_matching_checkout(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(BUILD, "git_head", lambda repo_root: "head-commit")
    monkeypatch.setattr(BUILD, "git_status", lambda repo_root: "")
    monkeypatch.setenv("GITHUB_SHA", "stale-commit")

    with pytest.raises(RuntimeError, match="does not match checkout HEAD"):
        BUILD.expected_build_commit(Path("."))


def test_dirty_development_build_is_explicitly_marked(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(BUILD, "git_head", lambda repo_root: "head-commit")
    monkeypatch.setattr(BUILD, "git_status", lambda repo_root: " M source.rs")
    monkeypatch.setenv("GITHUB_SHA", "stale-commit")

    assert (
        BUILD.expected_build_commit(Path("."), allow_dirty=True)
        == "head-commit-dirty"
    )


def test_clean_matching_sha_returns_head(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(BUILD, "git_head", lambda repo_root: "head-commit")
    monkeypatch.setattr(BUILD, "git_status", lambda repo_root: "")
    monkeypatch.setenv("GITHUB_SHA", "head-commit")

    assert BUILD.expected_build_commit(Path(".")) == "head-commit"


def test_explicit_expected_commit_overrides_synthetic_github_sha(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(BUILD, "git_head", lambda repo_root: "head-commit")
    monkeypatch.setattr(BUILD, "git_status", lambda repo_root: "")
    monkeypatch.setenv("GITHUB_SHA", "synthetic-merge-commit")
    monkeypatch.setenv("SKY_EXPECTED_BUILD_COMMIT", "head-commit")

    assert BUILD.expected_build_commit(Path(".")) == "head-commit"


def test_clean_without_reported_sha_returns_head(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(BUILD, "git_head", lambda repo_root: "head-commit")
    monkeypatch.setattr(BUILD, "git_status", lambda repo_root: "")
    monkeypatch.delenv("GITHUB_SHA", raising=False)

    assert BUILD.expected_build_commit(Path(".")) == "head-commit"


def test_dirty_release_build_is_rejected(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(BUILD, "git_head", lambda repo_root: "head-commit")
    monkeypatch.setattr(BUILD, "git_status", lambda repo_root: "?? new.rs")
    monkeypatch.delenv("GITHUB_SHA", raising=False)

    with pytest.raises(RuntimeError, match="requires a clean working tree"):
        BUILD.expected_build_commit(Path("."))


def test_main_returns_failure_when_provenance_gate_fails(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(BUILD, "expected_build_commit", lambda *args, **kwargs: (_ for _ in ()).throw(RuntimeError("stale")))
    monkeypatch.setattr(sys, "argv", ["build_rust_wheel.py"])

    assert BUILD.main() == 1


def test_wheel_build_does_not_generate_application_metadata() -> None:
    legacy_generator = "generate_" + "application_build_metadata"
    assert not hasattr(BUILD, legacy_generator)

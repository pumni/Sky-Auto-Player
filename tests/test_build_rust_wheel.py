from __future__ import annotations

import importlib.util
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

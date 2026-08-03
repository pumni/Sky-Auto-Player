"""Native extension smoke-test contract."""

from __future__ import annotations

import sys

import main as main_mod
from sky_music.orchestration.native_admission import RustBuildInfo


class _FakeSession:
    def __init__(self, *_args, **_kwargs) -> None:
        return None

    def start(self) -> None:
        return None

    def join(self, timeout_ms: int) -> bool:
        assert timeout_ms == 5_000
        return True

    def snapshot(self) -> dict[str, object]:
        return {"status": "finished"}


class _FakeConfig:
    def __init__(self, **_kwargs) -> None:
        return None


class _FakeNative:
    DispatchSession = _FakeSession
    SessionConfig = _FakeConfig


def test_rust_selftest_runs_empty_native_schedule_without_input(
    monkeypatch, capsys
) -> None:
    monkeypatch.setattr(
        main_mod,
        "require_rust_core",
        lambda: RustBuildInfo(
            native_build_commit="a" * 40,
            schema_version=2,
            native_abi="cp314t-win_amd64",
            native_version="0.1.0",
            rustc_version="rustc test",
            module_path="sky_player_rs.pyd",
            win32_backend=True,
        ),
    )
    monkeypatch.setattr(main_mod.sys, "frozen", False, raising=False)
    monkeypatch.setitem(sys.modules, "sky_player_rs", _FakeNative())

    assert main_mod._run_rust_selftest() == 0
    output = capsys.readouterr().out
    assert "runtime_contract=true" in output
    assert "release_contract=not_applicable" in output
    assert "sha_match" not in output
    assert "empty_session=true" in output


def test_rust_selftest_reports_frozen_release_contract(monkeypatch, capsys) -> None:
    monkeypatch.setattr(
        main_mod,
        "require_rust_core",
        lambda: RustBuildInfo(
            native_build_commit="a" * 40,
            schema_version=2,
            native_abi="cp314t-win_amd64",
            native_version="0.1.0",
            rustc_version="rustc test",
            module_path="sky_player_rs.pyd",
            win32_backend=True,
            app_build_commit="a" * 40,
            release_commit_match=True,
        ),
    )
    monkeypatch.setattr(main_mod.sys, "frozen", True, raising=False)
    monkeypatch.setitem(sys.modules, "sky_player_rs", _FakeNative())

    assert main_mod._run_rust_selftest() == 0
    output = capsys.readouterr().out
    assert "runtime_contract=true" in output
    assert "release_contract=true" in output
    assert "application_commit=" + ("a" * 40) in output
    assert "sha_match=true" in output


def test_rust_selftest_branch_is_wired() -> None:
    assert "--selftest-rust" in main_mod.main.__code__.co_consts

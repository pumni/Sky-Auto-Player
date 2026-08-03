from __future__ import annotations

import sys
from types import ModuleType, SimpleNamespace

from sky_music.infrastructure import doctor

APP_COMMIT = "a" * 40


def test_native_doctor_reports_build_metadata(monkeypatch) -> None:
    info = {
        "rust_core_version": "0.1.0",
        "rustc_version": "rustc 1.97.1",
        "pyo3_version": "0.29.0",
        "native_abi": "cp314t-win_amd64",
        "schema_version": 2,
        "native_schema_version": 2,
        "native_build_commit": APP_COMMIT,
        "version": "0.1.0",
        "free_threaded": True,
        "win32_backend": True,
    }
    monkeypatch.setitem(
        sys.modules,
        "sky_player_rs",
        SimpleNamespace(build_info=lambda: info),
    )
    monkeypatch.setitem(sys.modules, "sky_music._native_build", None)
    monkeypatch.setattr(doctor.sys, "frozen", False, raising=False)
    monkeypatch.setattr(doctor.sys, "_is_gil_enabled", lambda: False)

    result = doctor.check_native_dispatch()

    assert result["ok"] is True
    assert result["required"] is True
    assert "rustc 1.97.1" in result["msg"]
    assert "cp314t-win_amd64" in result["msg"]
    assert result["mode"] == "source development"
    assert result["commit_match"] is None
    assert result["release_contract"] == "not applicable"


def test_native_doctor_reports_frozen_release_contract(monkeypatch) -> None:
    info = {
        "rustc_version": "rustc 1.97.1",
        "native_abi": "cp314t-win_amd64",
        "schema_version": 2,
        "native_schema_version": 2,
        "native_build_commit": APP_COMMIT,
        "version": "0.1.0",
        "free_threaded": True,
        "win32_backend": True,
    }
    monkeypatch.setitem(
        sys.modules,
        "sky_player_rs",
        SimpleNamespace(build_info=lambda: info),
    )
    app_metadata = ModuleType("sky_music._native_build")
    app_metadata.APP_BUILD_COMMIT = APP_COMMIT  # type: ignore[attr-defined]
    monkeypatch.setitem(sys.modules, "sky_music._native_build", app_metadata)
    monkeypatch.setattr(doctor.sys, "frozen", True, raising=False)
    monkeypatch.setattr(doctor.sys, "_is_gil_enabled", lambda: False)

    result = doctor.check_native_dispatch()

    assert result["ok"] is True
    assert result["mode"] == "frozen production"
    assert result["application_build_commit"] == APP_COMMIT
    assert result["commit_match"] is True
    assert result["release_contract"] == "PASS"


def test_native_doctor_marks_explicit_missing_module_as_required(monkeypatch) -> None:
    monkeypatch.delitem(sys.modules, "sky_player_rs", raising=False)
    real_import = __import__

    def reject_native(name: str, *args, **kwargs):
        if name == "sky_player_rs":
            raise ImportError("not installed")
        return real_import(name, *args, **kwargs)

    monkeypatch.setattr("builtins.__import__", reject_native)

    result = doctor.check_native_dispatch()

    assert result["ok"] is False
    assert result["required"] is True
    assert "is required" in result["msg"]

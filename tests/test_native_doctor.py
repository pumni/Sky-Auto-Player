from __future__ import annotations

import sys
from types import SimpleNamespace

from sky_music.infrastructure import doctor
from sky_music.orchestration import native_dispatch


def test_native_doctor_reports_build_metadata(monkeypatch) -> None:
    info = {
        "rust_core_version": "0.1.0",
        "rustc_version": "rustc 1.97.1",
        "pyo3_version": "0.29.0",
        "native_abi": "cp314t-win_amd64",
        "native_schema_version": 1,
        "native_build_commit": "abc123",
    }
    monkeypatch.setitem(
        sys.modules,
        "sky_player_rs",
        SimpleNamespace(build_info=lambda: info),
    )
    monkeypatch.setattr(
        native_dispatch,
        "probe_native_dispatch",
        lambda **_kwargs: native_dispatch.NativeProbeResult(
            available=True,
            reason=native_dispatch.NativeProbeReason.AVAILABLE,
            detail="test native dispatch",
        ),
    )

    result = doctor.check_native_dispatch()

    assert result["ok"] is True
    assert result["required"] is True
    assert "rustc 1.97.1" in result["msg"]
    assert "cp314t-win_amd64" in result["msg"]


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

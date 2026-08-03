from __future__ import annotations

import sys
from types import ModuleType, SimpleNamespace

import pytest

from sky_music.orchestration import native_admission

APP_COMMIT = "a" * 40
BASE_INFO: dict[str, object] = {
    "version": "0.1.0",
    "rust_core_version": "0.1.0",
    "rustc_version": "rustc 1.97.1",
    "schema_version": 2,
    "native_schema_version": 2,
    "native_abi": "cp314t-win_amd64",
    "native_build_commit": APP_COMMIT,
    "free_threaded": True,
    "win32_backend": True,
}


def test_exact_native_contract_passes() -> None:
    result = native_admission.validate_rust_build_info(
        app_commit=APP_COMMIT,
        native_info=BASE_INFO,
    )

    assert result.app_build_commit == APP_COMMIT
    assert result.native_build_commit == APP_COMMIT
    assert result.schema_version == 2
    assert result.native_abi == "cp314t-win_amd64"
    assert result.win32_backend is True


@pytest.mark.parametrize(
    ("field", "value", "match"),
    [
        ("app_commit", "", "application commit"),
        ("app_commit", "abc123", "40-character Git SHA"),
        ("app_commit", APP_COMMIT.upper(), "40-character Git SHA"),
        ("native_build_commit", None, "native commit"),
        ("native_build_commit", "", "native commit"),
        ("native_build_commit", "abc123", "40-character Git SHA"),
        ("native_build_commit", "unknown", "40-character Git SHA"),
        ("native_build_commit", f"{APP_COMMIT}-dirty", "40-character Git SHA"),
        ("schema_version", 1, "schema mismatch"),
        ("native_schema_version", 1, "native schema mismatch"),
        ("native_abi", "cp313-win_amd64", "ABI mismatch"),
        ("free_threaded", False, "free-threaded"),
        ("win32_backend", False, "Win32 SendInput"),
        ("rustc_version", 1971, "rustc_version"),
        ("version", None, "native version"),
    ],
)
def test_invalid_native_contract_is_rejected(
    field: str, value: object, match: str
) -> None:
    info = dict(BASE_INFO)
    if field == "app_commit":
        app_commit = value
    else:
        info[field] = value
        app_commit = APP_COMMIT

    with pytest.raises(native_admission.NativeAdmissionError, match=match):
        native_admission.validate_rust_build_info(
            app_commit=app_commit,  # type: ignore[arg-type]
            native_info=info,
        )


@pytest.mark.parametrize(
    "field",
    [
        "native_build_commit",
        "schema_version",
        "native_schema_version",
        "native_abi",
        "version",
        "rustc_version",
        "free_threaded",
        "win32_backend",
    ],
)
def test_missing_native_metadata_is_rejected(field: str) -> None:
    info = dict(BASE_INFO)
    del info[field]

    with pytest.raises(native_admission.NativeAdmissionError):
        native_admission.validate_rust_build_info(
            app_commit=APP_COMMIT,
            native_info=info,
        )


def test_commit_mismatch_is_rejected() -> None:
    info = dict(BASE_INFO)
    info["native_build_commit"] = "b" * 40

    with pytest.raises(native_admission.NativeAdmissionError, match="does not match"):
        native_admission.validate_rust_build_info(
            app_commit=APP_COMMIT,
            native_info=info,
        )


def test_require_rust_core_calls_build_info_once(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls = 0

    def build_info() -> dict[str, object]:
        nonlocal calls
        calls += 1
        return BASE_INFO

    native_module = SimpleNamespace(
        __file__="sky_player_rs.pyd",
        build_info=build_info,
    )
    app_metadata = ModuleType("sky_music._native_build")
    app_metadata.APP_BUILD_COMMIT = APP_COMMIT  # type: ignore[attr-defined]
    monkeypatch.setitem(sys.modules, "sky_player_rs", native_module)
    monkeypatch.setitem(sys.modules, "sky_music._native_build", app_metadata)
    monkeypatch.setattr(native_admission.sys, "_is_gil_enabled", lambda: False)

    result = native_admission.require_rust_core()

    assert result.native_build_commit == APP_COMMIT
    assert calls == 1


def test_require_rust_core_fails_when_application_metadata_is_missing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setitem(sys.modules, "sky_music._native_build", None)

    with pytest.raises(native_admission.NativeAdmissionError, match="metadata is missing"):
        native_admission.require_rust_core()


def test_main_admission_happens_before_song_discovery(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import main

    monkeypatch.setattr(
        main,
        "require_rust_core",
        lambda: (_ for _ in ()).throw(
            native_admission.NativeAdmissionError("contract mismatch")
        ),
    )
    monkeypatch.setattr(
        main,
        "get_song_choices",
        lambda **_kwargs: pytest.fail("song discovery must follow native admission"),
    )
    monkeypatch.setattr("sys.argv", ["main.py", "--song", "not-used"])

    assert main.main() == 2

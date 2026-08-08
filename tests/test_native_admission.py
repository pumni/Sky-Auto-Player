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
    "schema_version": 4,
    "native_schema_version": 4,
    "native_abi": "cp314t-win_amd64",
    "native_build_commit": APP_COMMIT,
    "free_threaded": True,
    "win32_backend": True,
}


def _install_native(
    monkeypatch: pytest.MonkeyPatch,
    info: dict[str, object] | None = None,
) -> None:
    monkeypatch.setitem(
        sys.modules,
        "sky_player_rs",
        SimpleNamespace(
            __file__="sky_player_rs.pyd",
            build_info=lambda: dict(BASE_INFO if info is None else info),
        ),
    )
    monkeypatch.setattr(native_admission.sys, "_is_gil_enabled", lambda: False)


def test_runtime_validation_accepts_clean_dirty_and_arbitrary_development_ids() -> None:
    for commit in (APP_COMMIT, f"{APP_COMMIT}-dirty", "local-wheel-build"):
        info = dict(BASE_INFO)
        info["native_build_commit"] = commit

        result = native_admission.validate_native_runtime_info(native_info=info)

        assert result.native_build_commit == commit
        assert result.app_build_commit is None
        assert result.release_commit_match is None


@pytest.mark.parametrize(
    ("field", "value", "match"),
    [
        ("native_build_commit", None, "native commit"),
        ("native_build_commit", "", "native commit"),
        ("schema_version", 1, "schema mismatch"),
        ("native_schema_version", 1, "native schema mismatch"),
        ("native_abi", "cp313-win_amd64", "ABI mismatch"),
        ("free_threaded", False, "free-threaded"),
        ("win32_backend", False, "Win32 SendInput"),
        ("rustc_version", 1971, "rustc_version"),
        ("version", None, "native version"),
    ],
)
def test_runtime_validation_rejects_invalid_metadata(
    field: str, value: object, match: str
) -> None:
    info = dict(BASE_INFO)
    info[field] = value

    with pytest.raises(native_admission.NativeAdmissionError, match=match):
        native_admission.validate_native_runtime_info(native_info=info)


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
def test_runtime_validation_rejects_missing_metadata(field: str) -> None:
    info = dict(BASE_INFO)
    del info[field]

    with pytest.raises(native_admission.NativeAdmissionError):
        native_admission.validate_native_runtime_info(native_info=info)


@pytest.mark.parametrize(
    ("app_commit", "native_commit", "match"),
    [
        (APP_COMMIT, APP_COMMIT, None),
        (APP_COMMIT, "b" * 40, "does not match"),
        ("", APP_COMMIT, "application commit"),
        ("abc123", APP_COMMIT, "40-character Git SHA"),
        (APP_COMMIT.upper(), APP_COMMIT, "40-character Git SHA"),
        (f"{APP_COMMIT}-dirty", APP_COMMIT, "40-character Git SHA"),
        (APP_COMMIT, "", "native commit"),
        (APP_COMMIT, "abc123", "40-character Git SHA"),
        (APP_COMMIT, APP_COMMIT.upper(), "40-character Git SHA"),
        (APP_COMMIT, f"{APP_COMMIT}-dirty", "40-character Git SHA"),
        (APP_COMMIT, "unknown", "40-character Git SHA"),
    ],
)
def test_release_commit_validation(
    app_commit: str, native_commit: str, match: str | None
) -> None:
    if match is None:
        native_admission.validate_release_commit(
            app_commit=app_commit,
            native_commit=native_commit,
        )
        return

    with pytest.raises(native_admission.NativeAdmissionError, match=match):
        native_admission.validate_release_commit(
            app_commit=app_commit,
            native_commit=native_commit,
        )


def test_source_require_does_not_read_packaged_metadata_or_compare_sha(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    info = dict(BASE_INFO)
    info["native_build_commit"] = f"{APP_COMMIT}-dirty"
    _install_native(monkeypatch, info)
    monkeypatch.setattr(native_admission.sys, "frozen", False, raising=False)
    monkeypatch.setitem(sys.modules, "sky_music._native_build", None)

    result = native_admission.require_rust_core()

    assert result.native_build_commit == f"{APP_COMMIT}-dirty"
    assert result.app_build_commit is None
    assert result.release_commit_match is None


def test_source_require_fails_on_gil_enabled_runtime(monkeypatch: pytest.MonkeyPatch) -> None:
    _install_native(monkeypatch)
    monkeypatch.setattr(native_admission.sys, "frozen", False, raising=False)
    monkeypatch.setattr(native_admission.sys, "_is_gil_enabled", lambda: True)

    with pytest.raises(native_admission.NativeAdmissionError, match="not free-threaded"):
        native_admission.require_rust_core()


def test_frozen_require_validates_and_stores_release_commit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _install_native(monkeypatch)
    monkeypatch.setattr(native_admission.sys, "frozen", True, raising=False)
    app_metadata = ModuleType("sky_music._native_build")
    app_metadata.APP_BUILD_COMMIT = APP_COMMIT  # type: ignore[attr-defined]
    monkeypatch.setitem(sys.modules, "sky_music._native_build", app_metadata)

    result = native_admission.require_rust_core()

    assert result.app_build_commit == APP_COMMIT
    assert result.release_commit_match is True


@pytest.mark.parametrize(
    ("native_commit", "metadata", "match"),
    [
        (APP_COMMIT, None, "metadata is missing"),
        ("b" * 40, APP_COMMIT, "does not match"),
        (f"{APP_COMMIT}-dirty", APP_COMMIT, "40-character Git SHA"),
    ],
)
def test_frozen_require_fails_closed_on_release_metadata(
    monkeypatch: pytest.MonkeyPatch,
    native_commit: str,
    metadata: str | None,
    match: str,
) -> None:
    info = dict(BASE_INFO)
    info["native_build_commit"] = native_commit
    _install_native(monkeypatch, info)
    monkeypatch.setattr(native_admission.sys, "frozen", True, raising=False)
    if metadata is None:
        monkeypatch.setitem(sys.modules, "sky_music._native_build", None)
    else:
        app_metadata = ModuleType("sky_music._native_build")
        app_metadata.APP_BUILD_COMMIT = metadata  # type: ignore[attr-defined]
        monkeypatch.setitem(sys.modules, "sky_music._native_build", app_metadata)

    with pytest.raises(native_admission.NativeAdmissionError, match=match):
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

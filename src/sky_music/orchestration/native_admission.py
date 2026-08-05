"""Fail-closed startup admission for the Rust production dispatch core."""

from __future__ import annotations

import re
import sys
from collections.abc import Mapping
from dataclasses import dataclass, replace
from typing import cast

from sky_music.orchestration.native_models import RUST_DISPATCH_SCHEMA_VERSION

EXPECTED_NATIVE_ABI = "cp314t-win_amd64"
_FULL_GIT_SHA = re.compile(r"^[0-9a-f]{40}$")


@dataclass(frozen=True, slots=True)
class RustBuildInfo:
    """Validated native metadata retained by the application startup path."""

    native_build_commit: str
    schema_version: int
    native_abi: str
    native_version: str
    rustc_version: str
    module_path: str
    win32_backend: bool
    app_build_commit: str | None = None
    release_commit_match: bool | None = None


class NativeAdmissionError(RuntimeError):
    """The packaged application cannot safely admit the Rust core."""


@dataclass(frozen=True, slots=True)
class NativeInspection:
    """Raw native metadata used by doctor diagnostics, not playback."""

    module_path: str | None
    info: dict[str, object] | None
    error: str | None


def _require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise NativeAdmissionError(f"{field} is missing or not a non-empty string")
    return value.strip()


def _require_int(value: object, field: str) -> int:
    if type(value) is not int:
        raise NativeAdmissionError(f"{field} is missing or not an integer")
    return value


def _require_bool(value: object, field: str) -> bool:
    if type(value) is not bool:
        raise NativeAdmissionError(f"{field} is missing or not a boolean")
    return value


def _require_sha(value: object, field: str) -> str:
    commit = _require_text(value, field)
    if _FULL_GIT_SHA.fullmatch(commit) is None:
        raise NativeAdmissionError(
            f"{field} must be a lowercase 40-character Git SHA; got {commit!r}"
        )
    return commit


def validate_native_runtime_info(
    *, native_info: Mapping[str, object]
) -> RustBuildInfo:
    """Validate native compatibility without release provenance checks."""

    native_commit = _require_text(native_info.get("native_build_commit"), "native commit")
    schema_version = _require_int(native_info.get("schema_version"), "schema_version")
    native_schema_version = _require_int(
        native_info.get("native_schema_version"), "native_schema_version"
    )
    native_abi = _require_text(native_info.get("native_abi"), "native_abi")
    native_version = _require_text(native_info.get("version"), "native version")
    rustc_version = _require_text(native_info.get("rustc_version"), "rustc_version")
    free_threaded = _require_bool(native_info.get("free_threaded"), "free_threaded")
    win32_backend = _require_bool(native_info.get("win32_backend"), "win32_backend")

    if schema_version != RUST_DISPATCH_SCHEMA_VERSION:
        raise NativeAdmissionError(
            f"schema mismatch: expected {RUST_DISPATCH_SCHEMA_VERSION}, actual {schema_version}"
        )
    if native_schema_version != RUST_DISPATCH_SCHEMA_VERSION:
        raise NativeAdmissionError(
            "native schema mismatch: "
            f"expected {RUST_DISPATCH_SCHEMA_VERSION}, actual {native_schema_version}"
        )
    if native_abi != EXPECTED_NATIVE_ABI:
        raise NativeAdmissionError(
            f"ABI mismatch: expected {EXPECTED_NATIVE_ABI}, actual {native_abi}"
        )
    if free_threaded is not True:
        raise NativeAdmissionError("native extension is not built for free-threaded CPython")
    if win32_backend is not True:
        raise NativeAdmissionError("native extension does not expose the Win32 SendInput backend")

    return RustBuildInfo(
        native_build_commit=native_commit,
        schema_version=schema_version,
        native_abi=native_abi,
        native_version=native_version,
        rustc_version=rustc_version,
        module_path=str(native_info.get("module_path", "")),
        win32_backend=win32_backend,
    )


def validate_release_commit(*, app_commit: str, native_commit: str) -> None:
    """Validate exact application/native provenance for frozen production."""

    expected = _require_sha(app_commit, "application commit")
    actual = _require_sha(native_commit, "native commit")
    if actual != expected:
        raise NativeAdmissionError(
            "native commit does not match application commit: "
            f"expected {expected}, actual {actual}"
        )


def _packaged_application_commit() -> str:
    """Load application provenance embedded by the frozen build pipeline."""

    try:
        from sky_music._native_build import (
            APP_BUILD_COMMIT,
        )
    except (AttributeError, ImportError) as exc:
        raise NativeAdmissionError(
            "application build metadata is missing; run the native/build pipeline first"
        ) from exc
    return APP_BUILD_COMMIT


def inspect_rust_core() -> NativeInspection:
    """Collect native metadata for doctor without admitting a playback session."""

    try:
        import sky_player_rs  # type: ignore[import-not-found]
    except (ImportError, OSError, RuntimeError) as exc:
        return NativeInspection(None, None, f"cannot import sky_player_rs: {exc}")

    module_path = getattr(sky_player_rs, "__file__", None)
    build_info = getattr(sky_player_rs, "build_info", None)
    if not callable(build_info):
        return NativeInspection(
            str(module_path) if module_path is not None else None,
            None,
            "sky_player_rs.build_info() is missing",
    )
    try:
        raw_info = cast(object, build_info())
        if not isinstance(raw_info, Mapping):
            raise TypeError("sky_player_rs.build_info() did not return a mapping")
        info: dict[str, object] = {
            str(key): value for key, value in raw_info.items()
        }
    except (OSError, RuntimeError, TypeError, ValueError) as exc:
        return NativeInspection(
            str(module_path) if module_path is not None else None,
            None,
            f"sky_player_rs.build_info() failed: {type(exc).__name__}: {exc}",
        )
    info["module_path"] = str(module_path) if module_path is not None else ""
    return NativeInspection(
        str(module_path) if module_path is not None else None,
        info,
        None,
    )


def require_rust_core() -> RustBuildInfo:
    """Admit the Rust core once at application startup, or fail closed."""

    inspection = inspect_rust_core()
    if inspection.error is not None or inspection.info is None:
        raise NativeAdmissionError(inspection.error or "native metadata is unavailable")

    runtime_gil_probe = getattr(sys, "_is_gil_enabled", None)
    if not callable(runtime_gil_probe) or runtime_gil_probe():
        raise NativeAdmissionError("active Python runtime is not free-threaded")

    try:
        result = validate_native_runtime_info(native_info=inspection.info)
        if not getattr(sys, "frozen", False):
            return result

        app_commit = _packaged_application_commit()
        validate_release_commit(
            app_commit=app_commit,
            native_commit=result.native_build_commit,
        )
        return replace(
            result,
            app_build_commit=app_commit,
            release_commit_match=True,
        )
    except NativeAdmissionError as exc:
        module_path = inspection.module_path or "<unknown>"
        raise NativeAdmissionError(f"{exc} (module: {module_path})") from exc


__all__ = [
    "EXPECTED_NATIVE_ABI",
    "NativeAdmissionError",
    "NativeInspection",
    "RustBuildInfo",
    "inspect_rust_core",
    "require_rust_core",
    "validate_native_runtime_info",
    "validate_release_commit",
]

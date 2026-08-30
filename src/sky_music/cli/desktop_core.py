"""Entrypoint for the bounded Python Desktop Core worker."""

from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any

from sky_music.config import load_config
from sky_music.infrastructure.desktop_ipc.protocol import (
    bounded_text,
    event,
    write_frame,
)
from sky_music.infrastructure.desktop_ipc.server import DesktopCoreServer
from sky_music.infrastructure.realtime import assert_free_threaded_runtime
from sky_music.orchestration.catalog_service import CatalogService
from sky_music.orchestration.desktop_calibration import run_packaged_smoke_calibration
from sky_music.orchestration.native_admission import require_rust_core
from sky_music.orchestration.settings_service import SettingsService


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.add_argument("--desktop-worker", action="store_true")
    parser.add_argument("--parent-pid")
    parser.add_argument("--install-root")
    return parser


def _positive_pid(raw: str | None) -> int | None:
    if raw is None:
        return None
    try:
        value = int(raw, 10)
    except ValueError as exc:
        raise ValueError("--parent-pid must be a positive integer") from exc
    if value <= 0:
        raise ValueError("--parent-pid must be a positive integer")
    return value


def _install_root(raw: str | None) -> Path | None:
    if raw is None:
        return None
    if not raw or "\x00" in raw or len(raw) > 4096:
        raise ValueError("--install-root is invalid")
    path = Path(raw)
    if not path.is_absolute():
        raise ValueError("--install-root must be an absolute path")
    return path.resolve(strict=False)


def _fatal(stdout: Any, stderr: Any, code: str, message: object) -> None:
    print(f"desktop Core fatal: {message}", file=stderr)
    try:
        write_frame(
            stdout,
            event("core.fatal", {"code": bounded_text(code), "message": bounded_text(message)}),
        )
    except Exception as exc:
        print(f"desktop Core could not emit fatal event: {exc}", file=stderr)


def run_desktop_core(
    argv: list[str] | None = None,
    *,
    stdin: Any = None,
    stdout: Any = None,
    stderr: Any = None,
    runtime_guard: Callable[[], None] | None = None,
    native_admission: Callable[[], Any] | None = None,
) -> int:
    """Validate startup, admit the native core, then serve bounded requests."""
    input_stream = stdin if stdin is not None else sys.stdin.buffer
    output_stream = stdout if stdout is not None else sys.stdout.buffer
    error_stream = stderr if stderr is not None else sys.stderr
    try:
        args = _parser().parse_args(argv)
        if not args.desktop_worker:
            raise ValueError("--desktop-worker is required")
        parent_pid = _positive_pid(args.parent_pid)
        install_root = _install_root(args.install_root)
        if install_root is not None:
            if not install_root.is_dir():
                raise ValueError("--install-root must name an existing directory")
            os.chdir(install_root)
            from sky_music.infrastructure.update_runtime import (
                active_update_for_install,
            )

            active_update = active_update_for_install(install_root)
            if active_update is not None:
                raise ValueError(
                    "an update transaction is active for this installation; "
                    "restart after the updater completes"
                )
        (runtime_guard or assert_free_threaded_runtime)()
        cfg = load_config()
        native_info = (native_admission or require_rust_core)()
        settings = SettingsService(cfg)
        catalog = CatalogService(settings.snapshot().songs_dir)
        server = DesktopCoreServer(
            settings_service=settings,
            catalog_service=catalog,
            native_build_info=native_info,
            parent_pid=parent_pid,
            install_root=install_root,
            calibration_runner=(
                run_packaged_smoke_calibration
                if os.environ.get("SKY_PACKAGED_SAFE_CALIBRATION") == "1"
                else None
            ),
        )
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
        _fatal(output_stream, error_stream, "startup_failed", "invalid Core arguments")
        return code or 2
    except Exception as exc:
        print(f"desktop Core startup failure: {exc}", file=error_stream)
        _fatal(output_stream, error_stream, "startup_failed", "Core startup admission failed")
        return 2
    return server.serve(input_stream, output_stream, stderr=error_stream)


def main(argv: list[str] | None = None) -> int:
    return run_desktop_core(argv)


__all__ = ["main", "run_desktop_core"]

"""Build and qualify the exact native portable Windows release candidate.

This is temporary repository tooling.  It assembles the Tauri shell and native
helpers directly. Wave 6 will replace this orchestration with ``xtask``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import zipfile
from collections.abc import Sequence
from contextlib import suppress
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from release_common import (  # noqa: E402
    cargo_release_build_command,
    get_git_head,
    get_project_version,
    native_build_environment,
    native_source_fingerprint,
    parse_native_metadata,
    validate_native_build_provenance,
    validate_observed_native_metadata,
    write_release_manifest,
)

APP_NAME = "Sky-Auto-Player"
VERSION = get_project_version()
PRIMARY_EXE = "Sky-Auto-Player.exe"
UPDATER_EXE = "Sky-Auto-Player-Updater.exe"
CALIBRATION_EXE = "native_calibration.exe"
RELEASE_NAME = f"{APP_NAME}-v{VERSION}"
ZIP_NAME = f"{RELEASE_NAME}.zip"


def _hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _run(command: Sequence[str], *, cwd: Path = ROOT, env: dict[str, str] | None = None) -> None:
    print(f"[portable-release] {' '.join(str(item) for item in command)}", flush=True)
    subprocess.run(list(command), cwd=str(cwd), env=env, check=True)


def _decode_output(value: object) -> str:
    """Decode child diagnostics deterministically, including invalid bytes."""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    if value is None:
        return ""
    return str(value)


def _read_gui_smoke_phase_log(path: Path) -> str:
    """Read the bounded startup/shutdown trace emitted by the packaged shell."""
    try:
        content = path.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        return f"<unavailable: {error}>"
    return content[-8000:] or "<empty>"


def _capture(command: Sequence[str], *, cwd: Path = ROOT) -> str:
    result = subprocess.run(
        list(command), cwd=str(cwd), capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def _capture_native_metadata(command: Sequence[str], *, label: str) -> dict[str, Any]:
    result = subprocess.run(
        list(command),
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"{label} metadata command failed ({result.returncode}): {result.stderr[-4000:]}"
        )
    return parse_native_metadata(result.stdout.strip(), label=label)


def observe_native_build_metadata(
    desktop_exe: Path,
    calibration_exe: Path,
    *,
    repo_head: str,
    version: str,
    source_fingerprint: str,
) -> str:
    """Read and qualify metadata from the exact binaries supplied by the builder."""
    desktop = _capture_native_metadata(
        [str(desktop_exe), "--selftest-build-info"], label="desktop"
    )
    calibration = _capture_native_metadata(
        [str(calibration_exe), "--metadata"], label="calibration"
    )
    return validate_observed_native_metadata(
        repo_head=repo_head,
        version=version,
        source_fingerprint=source_fingerprint,
        desktop_metadata=desktop,
        calibration_metadata=calibration,
    )


def _default_config() -> dict[str, Any]:
    return {
        "schema_version": 3,
        "theme": "aurora",
        "ui_background_mode": "transparent",
        "default_hold_frames": 1.0,
        "default_tempo_scale": 1.0,
        "game_fps": 60,
        "telemetry_enabled_by_default": False,
        "verbose_hud": False,
        "hotkeys": {
            "pause": "f8",
            "skip": "f9",
            "quit": "f10",
            "refocus": "f6",
            "panic": "ctrl+alt+backspace",
        },
        "safety": {"prompt_on_medium_risk": True, "prompt_on_high_risk": True},
        "songs_dir": "songs",
        "sky_process_names": ["Sky.exe", "Sky Children of the Light.exe"],
        "allow_title_fallback": False,
        "update": {
            "auto_check": True,
            "channel": "stable",
            "skip_version": "",
            "check_interval_s": 86400,
            "last_check_ts": 0,
            "last_error_ts": 0,
            "last_notified_version": "",
            "legacy_old_dir_sweep_pending": False,
        },
    }


def _copy_tree_contents(source: Path, destination: Path) -> None:
    if not source.is_dir():
        raise RuntimeError(f"required directory is missing: {source}")
    destination.mkdir(parents=True, exist_ok=True)
    for item in source.iterdir():
        target = destination / item.name
        if item.is_dir():
            shutil.copytree(item, target, dirs_exist_ok=True)
        elif item.is_file():
            shutil.copy2(item, target)
        else:
            raise RuntimeError(f"unsupported package input: {item}")


def _copy_file(source: Path, destination: Path) -> None:
    if not source.is_file():
        raise RuntimeError(f"required build output is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def _assert_no_path_escape(root: Path) -> None:
    root = root.resolve()
    seen: dict[str, str] = {}
    for path in root.rglob("*"):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise RuntimeError(f"release contains a symlink: {relative}")
        try:
            path.resolve().relative_to(root)
        except ValueError:
            raise RuntimeError(f"release path escapes root: {relative}") from None
        folded = relative.casefold()
        previous = seen.setdefault(folded, relative)
        if previous != relative:
            raise RuntimeError(f"case-colliding release paths: {previous}, {relative}")


def assemble_release(
    release_dir: Path,
    *,
    tauri_exe: Path,
    calibration_exe: Path,
    updater_exe: Path,
    songs_dir: Path,
    version: str,
    git_head: str,
    native_build_commit: str,
) -> Path:
    """Assemble one fresh release tree from exact build outputs."""
    if release_dir.exists():
        shutil.rmtree(release_dir)
    release_dir.mkdir(parents=True)
    _copy_file(tauri_exe, release_dir / PRIMARY_EXE)
    _copy_file(calibration_exe, release_dir / CALIBRATION_EXE)
    _copy_file(updater_exe, release_dir / UPDATER_EXE)
    _copy_tree_contents(songs_dir, release_dir / "songs")
    (release_dir / "config.json").write_text(
        json.dumps(_default_config(), indent=4) + "\n", encoding="utf-8"
    )
    readme = ROOT / "README.md"
    if readme.is_file():
        _copy_file(readme, release_dir / "README.md")
    _assert_no_path_escape(release_dir)
    assert_runtime_python_free(release_dir)
    write_release_manifest(
        release_dir,
        version,
        PRIMARY_EXE,
        git_head,
        dirty_worktree=False,
        native_build_commit=native_build_commit,
    )
    _assert_no_path_escape(release_dir)
    return release_dir


def assert_runtime_python_free(release_dir: Path) -> None:
    """Reject Python/sidecar files from the production portable tree."""
    forbidden: list[str] = []
    for path in release_dir.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(release_dir).as_posix()
        folded = relative.casefold()
        name = path.name.casefold()
        if (
            folded == ("sky-auto-player-" + "core.exe")
            or folded.startswith("_internal/")
            or name.startswith("python")
            or name == "base_library.zip"
            or path.suffix.casefold() in {".pyd", ".py", ".pyc"}
        ):
            forbidden.append(relative)
    if forbidden:
        raise RuntimeError(
            "production portable tree contains runtime Python artifacts: "
            f"{sorted(forbidden)}"
        )


def write_deterministic_zip(release_dir: Path, destination: Path) -> tuple[Path, str]:
    """Write a reproducible flat portable ZIP and its SHA-256 sidecar."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(
        destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as archive:
        for path in sorted(release_dir.rglob("*"), key=lambda item: item.relative_to(release_dir).as_posix()):
            if not path.is_file():
                continue
            relative = path.relative_to(release_dir).as_posix()
            info = zipfile.ZipInfo(relative, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, path.read_bytes())
    digest = _hash_file(destination)
    destination.with_name(destination.name + ".sha256").write_text(
        f"{digest}  {destination.name}\n", encoding="ascii"
    )
    return destination, digest


def write_provenance(
    destination: Path,
    *,
    repo_head: str,
    release_dir: Path,
    zip_path: Path,
    native_build_commit: str,
) -> dict[str, Any]:
    validate_native_build_provenance(repo_head, native_build_commit)
    manifest = release_dir / "MANIFEST.json"
    data: dict[str, Any] = {
        "schema_version": 1,
        "repo_head": repo_head,
        "version": VERSION,
        "tooling_python": {"version": platform.python_version(), "implementation": platform.python_implementation()},
        "rust": {"compiler": _capture(["rustc", "--version"])},
        "bun": {"version": _capture(["bun", "--version"])},
        "tauri": {"api": "2.11.1", "cli": "2.11.4", "runtime": "2.11.5"},
        "runtime_python": {"required": False, "bundled": False},
        "native_build_commit": native_build_commit,
        "artifact": {
            "filename": zip_path.name,
            "size": zip_path.stat().st_size,
            "sha256": _hash_file(zip_path),
            "manifest_sha256": _hash_file(manifest),
            "file_count": sum(1 for path in release_dir.rglob("*") if path.is_file()),
        },
        "portable_tree": str(release_dir.name),
    }
    destination.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    return data


def _runtime_smoke_env(*, cwd: Path, python_unavailable: bool) -> dict[str, str]:
    """Build a runtime environment that cannot discover repository Python."""

    env = os.environ.copy()
    if python_unavailable:
        system_root = Path(env.get("SystemRoot", r"C:\Windows"))
        env["PATH"] = os.pathsep.join(
            str(path)
            for path in (cwd, system_root / "System32", system_root, system_root / "System32" / "Wbem")
        )
        for name in (
            "PYTHONHOME",
            "PYTHONPATH",
            "VIRTUAL_ENV",
            "UV_PROJECT_ENVIRONMENT",
            "UV_PYTHON",
        ):
            env.pop(name, None)
    return env


def _run_tauri_pair_selftest(
    primary_exe: Path, *, cwd: Path, python_unavailable: bool = False
) -> None:
    # SafePackage is selected by the hidden selftest composition root in the
    # Rust binary.  No environment variable grants calibration/update bypass
    # authority to the normal production composition.
    env = _runtime_smoke_env(cwd=cwd, python_unavailable=python_unavailable)
    result = subprocess.run(
        [str(primary_exe), "--selftest-desktop-shell"],
        cwd=str(cwd),
        capture_output=True,
        text=False,
        env=env,
        timeout=60,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"packaged Tauri/Native Desktop selftest failed ({result.returncode}, "
            f"python_unavailable={python_unavailable}): "
            f"{_decode_output(result.stderr)[-4000:]}"
        )
    print(_decode_output(result.stdout).strip())


def _run_tauri_gui_smoke(
    primary_exe: Path, *, cwd: Path, python_unavailable: bool = False
) -> None:
    # The hidden GUI smoke entrypoint selects SafePackage explicitly in Rust;
    # PATH is only restricted here to prove runtime Python independence.
    env = _runtime_smoke_env(cwd=cwd, python_unavailable=python_unavailable)
    phase_log = cwd / "gui-smoke-phases.log"
    with suppress(OSError):
        phase_log.unlink(missing_ok=True)
    env["SKY_GUI_SMOKE_PHASE_LOG"] = str(phase_log)
    try:
        result = subprocess.run(
            [str(primary_exe), "--selftest-desktop-gui"],
            cwd=str(cwd),
            capture_output=True,
            text=False,
            env=env,
            timeout=60,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(
            f"packaged Tauri GUI smoke timed out (python_unavailable={python_unavailable}); "
            f"stdout={_decode_output(error.stdout)!r} "
            f"stderr={_decode_output(error.stderr)!r} "
            f"phase_log={_read_gui_smoke_phase_log(phase_log)!r}"
        ) from error
    if result.returncode != 0:
        raise RuntimeError(
            f"packaged Tauri GUI smoke failed ({result.returncode}, "
            f"python_unavailable={python_unavailable}): "
            f"{_decode_output(result.stderr)[-4000:]} "
            f"phase_log={_read_gui_smoke_phase_log(phase_log)!r}"
        )
    print(
        "Packaged Tauri GUI smoke phases:\n"
        f"{_read_gui_smoke_phase_log(phase_log)}"
    )
    print("Packaged Tauri GUI smoke: PASS")


def _run_exact_updater_qualification(output_root: Path, e2e_updater: Path) -> None:
    env = os.environ.copy()
    env["SKY_PORTABLE_ARTIFACT_DIR"] = str(output_root)
    env["SKY_PORTABLE_E2E_UPDATER"] = str(e2e_updater)
    _run(
        [
            "cargo",
            "test",
            "--manifest-path",
            str(ROOT / "rust" / "Cargo.toml"),
            "-p",
            "sky_updater",
            "--test",
            "portable_exact_artifact",
            "--all-features",
            "--locked",
            "--",
            "--test-threads=1",
        ],
        env=env,
    )


def _build() -> tuple[Path, Path, Path, Path]:
    if sys.platform != "win32":
        raise RuntimeError("portable release build requires Windows")
    git_head = get_git_head()
    version = get_project_version()
    if version != VERSION:
        raise RuntimeError("project version changed during build")
    env = native_build_environment()
    env["GITHUB_SHA"] = git_head
    env["SKY_NATIVE_BUILD_COMMIT"] = git_head
    source_fingerprint = native_source_fingerprint(ROOT)
    env["SKY_NATIVE_SOURCE_FINGERPRINT"] = source_fingerprint
    env["SKY_NATIVE_DIRTY_WORKTREE"] = "false"
    _run(["bun", "install", "--frozen-lockfile"], cwd=ROOT / "desktop")
    _run(["bun", "run", "build"], cwd=ROOT / "desktop")
    _run(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(ROOT / "rust" / "Cargo.toml"),
            "-p",
            "sky_desktop_shell",
            "--bin",
            "sky_desktop_shell",
            "--profile",
            "dist",
            "--locked",
        ],
        env=env,
    )
    _run(
        cargo_release_build_command(
            ROOT / "rust" / "crates" / "sky_dispatch_win32" / "Cargo.toml",
            "native_calibration",
        ),
        env=env,
    )
    # This disposable feature-gated runner is never copied into the release
    # tree. It qualifies updater transactions using a deterministic local source
    # while the shipped updater remains the production GitHub-only binary.
    _run(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(ROOT / "rust" / "crates" / "sky_updater" / "Cargo.toml"),
            "--bin",
            "sky_updater_e2e",
            "--profile",
            "dist",
            "--features",
            "e2e-local-source,e2e-fault-injection",
            "--locked",
        ],
        env=env,
    )
    _run(
        cargo_release_build_command(
            ROOT / "rust" / "crates" / "sky_updater" / "Cargo.toml", "sky_updater"
        ),
        env=env,
    )
    return (
        ROOT / "rust" / "target" / "dist" / "sky_desktop_shell.exe",
        ROOT / "rust" / "target" / "dist" / CALIBRATION_EXE,
        ROOT / "rust" / "target" / "dist" / "sky_updater.exe",
        ROOT / "rust" / "target" / "dist" / "sky_updater_e2e.exe",
    )


def run_pipeline(output_root: Path) -> Path:
    repo_head = get_git_head()
    tauri_exe, calibration_exe, updater_exe, e2e_updater_exe = _build()
    source_fingerprint = native_source_fingerprint(ROOT)
    built_native_commit = observe_native_build_metadata(
        tauri_exe,
        calibration_exe,
        repo_head=repo_head,
        version=VERSION,
        source_fingerprint=source_fingerprint,
    )
    release_dir = output_root / RELEASE_NAME
    assemble_release(
        release_dir,
        tauri_exe=tauri_exe,
        calibration_exe=calibration_exe,
        updater_exe=updater_exe,
        songs_dir=ROOT / "songs",
        version=VERSION,
        git_head=repo_head,
        native_build_commit=built_native_commit,
    )
    copied_native_commit = observe_native_build_metadata(
        release_dir / PRIMARY_EXE,
        release_dir / CALIBRATION_EXE,
        repo_head=repo_head,
        version=VERSION,
        source_fingerprint=source_fingerprint,
    )
    verifier = ROOT / "scripts" / "verify_release_manifest.py"
    _run(
        [sys.executable, str(verifier), "--release-dir", str(release_dir), "--version", VERSION]
    )
    smoke_root = Path(tempfile.mkdtemp(prefix="Sky Auto Player portable "))
    try:
        smoke_dir = smoke_root / release_dir.name
        shutil.copytree(release_dir, smoke_dir)
        assert_runtime_python_free(smoke_dir)
        _run_tauri_pair_selftest(smoke_dir / PRIMARY_EXE, cwd=smoke_dir)
        _run_tauri_pair_selftest(
            smoke_dir / PRIMARY_EXE, cwd=smoke_dir, python_unavailable=True
        )
        _run_tauri_gui_smoke(smoke_dir / PRIMARY_EXE, cwd=smoke_dir)
        _run_tauri_gui_smoke(
            smoke_dir / PRIMARY_EXE, cwd=smoke_dir, python_unavailable=True
        )
    finally:
        shutil.rmtree(smoke_root, ignore_errors=True)
    zip_path, _ = write_deterministic_zip(release_dir, output_root / ZIP_NAME)
    (output_root / "MANIFEST.json").write_bytes((release_dir / "MANIFEST.json").read_bytes())
    _run_exact_updater_qualification(output_root, e2e_updater_exe)
    provenance = write_provenance(
        output_root / "PROVENANCE.json",
        repo_head=repo_head,
        release_dir=release_dir,
        zip_path=zip_path,
        native_build_commit=copied_native_commit,
    )
    (output_root / "PORTABLE_ARTIFACT_SUMMARY.json").write_text(
        json.dumps(
            {
                "repo_head": repo_head,
                "artifact_name": zip_path.name,
                "artifact_size": provenance["artifact"]["size"],
                "artifact_sha256": provenance["artifact"]["sha256"],
                "manifest_sha256": provenance["artifact"]["manifest_sha256"],
                "portable_file_count": provenance["artifact"]["file_count"],
                "managed_entry_count": len(
                    json.loads((release_dir / "MANIFEST.json").read_text(encoding="utf-8"))["files"]
                ),
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    (output_root / "PORTABLE_QUALIFICATION.json").write_text(
        json.dumps(
            {
                "repo_head": repo_head,
                "artifact_sha256": provenance["artifact"]["sha256"],
                "target": "3.5.0",
                "previous_stable": "3.4.5",
                "exact_artifact_update": "passed",
                "exact_artifact_fault_rollback": "passed",
                "package_selftest": "passed",
                "python_unavailable_selftest": "passed",
                "native_runtime_negative_matrix": "passed",
                "tauri_native_runtime_smoke": "passed",
                "tauri_gui_smoke": "passed",
                "python_unavailable_gui_smoke": "passed",
                "packaged_updater_identity_smoke": "passed",
                "packaged_updater_ready_parent_smoke": "passed",
                "exact_artifact_transaction_restart": "qualified_with_local_source_runner",
                "production_updater_network_transaction": (
                    "not_run_without_public_release_or_local_http_source"
                ),
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return release_dir


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-root", type=Path, default=ROOT / "dist" / "portable")
    args = parser.parse_args(argv)
    if args.output_root.exists():
        # This is a narrow generated output directory, never a repository path.
        shutil.rmtree(args.output_root)
    args.output_root.mkdir(parents=True, exist_ok=True)
    release_dir = run_pipeline(args.output_root.resolve())
    print(f"Portable release artifact ready: {release_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

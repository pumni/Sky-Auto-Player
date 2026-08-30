"""Build and qualify the exact v4 portable Windows release candidate.

This is the Phase 8 release entry point.  It reuses the native build and
manifest helpers from ``src/build_app.py`` but assembles the Tauri shell and
the single Core runtime into the canonical portable layout.
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
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

import build_app  # noqa: E402

APP_NAME = "Sky-Auto-Player"
VERSION = build_app.get_project_version()
CORE_EXE = "Sky-Auto-Player-Core.exe"
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
    print(f"[phase8] {' '.join(str(item) for item in command)}", flush=True)
    subprocess.run(list(command), cwd=str(cwd), env=env, check=True)


def _decode_output(value: object) -> str:
    """Decode child diagnostics deterministically, including invalid bytes."""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    if value is None:
        return ""
    return str(value)


def _capture(command: Sequence[str], *, cwd: Path = ROOT) -> str:
    result = subprocess.run(
        list(command), cwd=str(cwd), capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


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
    core_dist: Path,
    calibration_exe: Path,
    updater_exe: Path,
    songs_dir: Path,
    version: str,
    git_head: str,
) -> Path:
    """Assemble one fresh release tree from exact build outputs."""
    if release_dir.exists():
        shutil.rmtree(release_dir)
    release_dir.mkdir(parents=True)
    _copy_file(tauri_exe, release_dir / PRIMARY_EXE)
    _copy_file(core_dist / CORE_EXE, release_dir / CORE_EXE)
    _copy_tree_contents(core_dist / "_internal", release_dir / "_internal")
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
    build_app.write_release_manifest(
        release_dir,
        version,
        PRIMARY_EXE,
        git_head,
        dirty_worktree=False,
        native_build_commit=git_head,
    )
    _assert_no_path_escape(release_dir)
    return release_dir


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
    manifest = release_dir / "MANIFEST.json"
    data: dict[str, Any] = {
        "schema_version": 1,
        "repo_head": repo_head,
        "version": VERSION,
        "python": {
            "version": platform.python_version(),
            "implementation": platform.python_implementation(),
            "free_threaded": not bool(getattr(sys, "_is_gil_enabled", lambda: True)()),
        },
        "rust": {"compiler": _capture(["rustc", "--version"])},
        "bun": {"version": _capture(["bun", "--version"])},
        "tauri": {"api": "2.11.1", "cli": "2.11.4", "runtime": "2.11.5"},
        "core": {"pyinstaller": _capture([sys.executable, "-c", "import PyInstaller; print(PyInstaller.__version__)"])},
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


def _run_core_selftest(core_exe: Path, *, cwd: Path) -> None:
    result = subprocess.run(
        [str(core_exe), "--selftest-desktop-core"],
        cwd=str(cwd),
        capture_output=True,
        text=False,
        timeout=60,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"packaged Core selftest failed ({result.returncode}): "
            f"{_decode_output(result.stderr)[-4000:]}"
        )
    print(_decode_output(result.stdout).strip())


def _run_core_selftest_negative_matrix(core_exe: Path, *, cwd: Path) -> None:
    """Run the packaged selftest against bounded deterministic bad children."""
    fixture = ROOT / "desktop" / "src-tauri" / "tests" / "fixtures" / "fake_core.py"
    if not fixture.is_file():
        raise RuntimeError(f"Core selftest fixture is missing: {fixture}")
    scenarios = (
        "startup_timeout",
        "eof_before_ready",
        "fatal_before_ready",
        "malformed",
        "non_utf8",
        "duplicate_output",
        "oversized_output",
        "request_timeout",
        "unknown_id",
        "force_shutdown",
    )
    for scenario in scenarios:
        env = os.environ.copy()
        env["SKY_PHASE8_SELFTEST_TIMEOUT_SECONDS"] = "0.1"
        env["SKY_PHASE8_SELFTEST_CHILD"] = json.dumps(
            [sys.executable, str(fixture), scenario]
        )
        result = subprocess.run(
            [str(core_exe), "--selftest-desktop-core"],
            cwd=str(cwd),
            env=env,
            capture_output=True,
            text=False,
            timeout=10,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            check=False,
        )
        if result.returncode == 0:
            raise RuntimeError(
                f"packaged Core selftest accepted bad child scenario {scenario}: "
                f"{_decode_output(result.stdout)}"
            )
    missing_env = os.environ.copy()
    missing_env["SKY_PHASE8_SELFTEST_CHILD"] = json.dumps(
        [str(cwd / "missing-core.exe")]
    )
    missing = subprocess.run(
        [str(core_exe), "--selftest-desktop-core"],
        cwd=str(cwd),
        env=missing_env,
        capture_output=True,
        text=False,
        timeout=10,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        check=False,
    )
    if missing.returncode == 0:
        raise RuntimeError("packaged Core selftest accepted a missing child")
    print(f"Packaged Desktop Core selftest negative matrix: {len(scenarios) + 1} PASS")


def _run_tauri_pair_selftest(primary_exe: Path, *, cwd: Path) -> None:
    result = subprocess.run(
        [str(primary_exe), "--selftest-desktop-shell"],
        cwd=str(cwd),
        capture_output=True,
        text=False,
        timeout=60,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"packaged Tauri/Core selftest failed ({result.returncode}): "
            f"{_decode_output(result.stderr)[-4000:]}"
        )
    print(_decode_output(result.stdout).strip())


def _run_tauri_gui_smoke(primary_exe: Path, *, cwd: Path) -> None:
    env = os.environ.copy()
    # The real production Core remains native-calibration-backed.  This
    # packaging-only environment selects the explicit no-input seam so the
    # actual Tauri/Core smoke can exercise calibration safely in CI.
    env["SKY_PHASE8_SAFE_CALIBRATION"] = "1"
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
            "packaged Tauri GUI smoke timed out; "
            f"stdout={_decode_output(error.stdout)!r} "
            f"stderr={_decode_output(error.stderr)!r}"
        ) from error
    if result.returncode != 0:
        raise RuntimeError(
            f"packaged Tauri GUI smoke failed ({result.returncode}): "
            f"{_decode_output(result.stderr)[-4000:]}"
        )
    print("Packaged Tauri GUI smoke: PASS")


def _run_tui_smoke(core_exe: Path, *, cwd: Path) -> None:
    result = subprocess.run(
        [str(core_exe), "--tui", "--list"],
        cwd=str(cwd),
        capture_output=True,
        text=False,
        timeout=60,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"packaged TUI smoke failed ({result.returncode}): "
            f"{_decode_output(result.stderr)[-4000:]}"
        )


def _run_exact_updater_qualification(output_root: Path, e2e_updater: Path) -> None:
    env = os.environ.copy()
    env["SKY_PHASE8_ARTIFACT_DIR"] = str(output_root)
    env["SKY_PHASE8_E2E_UPDATER"] = str(e2e_updater)
    _run(
        [
            "cargo",
            "test",
            "--manifest-path",
            str(ROOT / "rust" / "Cargo.toml"),
            "-p",
            "sky_updater",
            "--test",
            "phase8_exact_artifact",
            "--all-features",
            "--locked",
            "--",
            "--test-threads=1",
        ],
        env=env,
    )


def _build() -> tuple[Path, Path, Path, Path, Path]:
    if sys.platform != "win32":
        raise RuntimeError("Phase 8 portable build requires Windows")
    git_head = build_app.get_git_head()
    version = build_app.get_project_version()
    if version != VERSION:
        raise RuntimeError("project version changed during build")
    build_app.generate_version_py(version)
    build_app.generate_native_build_py(git_head)
    build_app.generate_version_info(version)
    env = build_app.native_build_environment()
    env["GITHUB_SHA"] = git_head
    env["SKY_NATIVE_BUILD_COMMIT"] = git_head
    from sky_music.orchestration.native_provenance import native_source_fingerprint

    env["SKY_NATIVE_SOURCE_FINGERPRINT"] = native_source_fingerprint(ROOT, "cp314t-win_amd64")
    env["SKY_NATIVE_DIRTY_WORKTREE"] = "false"
    try:
        _run([sys.executable, "scripts/build_rust_wheel.py"], env=env)
        build_app.verify_native_build_info(git_head)
        core_spec = ROOT / "Sky-Auto-Player-Core.spec"
        with build_app._source_bootloader_override():
            _run(
                [sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", str(core_spec)],
                env=env,
            )
        core_dist = ROOT / "dist" / "Sky-Auto-Player-Core"
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
                "--release",
                "--locked",
            ],
            env=env,
        )
        _run(
            build_app.cargo_release_build_command(
                ROOT / "rust" / "crates" / "sky_dispatch_win32" / "Cargo.toml",
                "native_calibration",
            ),
            env=env,
        )
        # This disposable feature-gated runner is never copied into the
        # release tree. It lets exact-artifact qualification use the native
        # updater transaction runner with a deterministic local source while
        # the shipped updater remains the production GitHub-only binary.
        _run(
            [
                "cargo",
                "build",
                "--manifest-path",
                str(ROOT / "rust" / "crates" / "sky_updater" / "Cargo.toml"),
                "--bin",
                "sky_updater_e2e",
                "--release",
                "--features",
                "e2e-local-source,e2e-fault-injection",
                "--locked",
            ],
            env=env,
        )
        # Build the shipped updater last so the exact release binary is
        # produced with the default (GitHub-only) feature set.
        _run(
            build_app.cargo_release_build_command(
                ROOT / "rust" / "crates" / "sky_updater" / "Cargo.toml", "sky_updater"
            ),
            env=env,
        )
        return (
            core_dist,
            ROOT / "rust" / "target" / "release" / "sky_desktop_shell.exe",
            ROOT / "rust" / "target" / "release" / CALIBRATION_EXE,
            ROOT / "rust" / "target" / "release" / "sky_updater.exe",
            ROOT / "rust" / "target" / "release" / "sky_updater_e2e.exe",
        )
    finally:
        build_app.VERSION_PY.unlink(missing_ok=True)
        build_app.NATIVE_BUILD_PY.unlink(missing_ok=True)


def run_pipeline(output_root: Path) -> Path:
    repo_head = build_app.get_git_head()
    core_dist, tauri_exe, calibration_exe, updater_exe, e2e_updater_exe = _build()
    release_dir = output_root / RELEASE_NAME
    assemble_release(
        release_dir,
        tauri_exe=tauri_exe,
        core_dist=core_dist,
        calibration_exe=calibration_exe,
        updater_exe=updater_exe,
        songs_dir=ROOT / "songs",
        version=VERSION,
        git_head=repo_head,
    )
    verifier = ROOT / "scripts" / "verify_release_manifest.py"
    _run(
        [sys.executable, str(verifier), "--release-dir", str(release_dir), "--version", VERSION]
    )
    smoke_root = Path(tempfile.mkdtemp(prefix="Sky Auto Player Phase8 "))
    try:
        smoke_dir = smoke_root / release_dir.name
        shutil.copytree(release_dir, smoke_dir)
        _run_core_selftest(smoke_dir / CORE_EXE, cwd=smoke_dir)
        _run_core_selftest_negative_matrix(smoke_dir / CORE_EXE, cwd=smoke_dir)
        _run_tauri_pair_selftest(smoke_dir / PRIMARY_EXE, cwd=smoke_dir)
        _run_tauri_gui_smoke(smoke_dir / PRIMARY_EXE, cwd=smoke_dir)
        _run_tui_smoke(smoke_dir / CORE_EXE, cwd=smoke_dir)
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
        native_build_commit=repo_head,
    )
    (output_root / "PHASE8_ARTIFACT_SUMMARY.json").write_text(
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
    (output_root / "PHASE8_QUALIFICATION.json").write_text(
        json.dumps(
            {
                "repo_head": repo_head,
                "artifact_sha256": provenance["artifact"]["sha256"],
                "target": "3.5.0",
                "previous_stable": "3.4.5",
                "exact_artifact_update": "passed",
                "exact_artifact_fault_rollback": "passed",
                "package_selftest": "passed",
                "core_selftest_negative_matrix": "passed",
                "tauri_core_pair_smoke": "passed",
                "tauri_gui_smoke": "passed",
                "tui_smoke": "passed",
                "packaged_updater_identity_smoke": "passed",
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
    parser.add_argument("--output-root", type=Path, default=ROOT / "dist" / "phase8")
    args = parser.parse_args(argv)
    if args.output_root.exists():
        # This is a narrow generated output directory, never a repository path.
        shutil.rmtree(args.output_root)
    args.output_root.mkdir(parents=True, exist_ok=True)
    release_dir = run_pipeline(args.output_root.resolve())
    print(f"Phase 8 artifact ready: {release_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

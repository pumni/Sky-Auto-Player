import json
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest


def test_write_release_manifest_covers_exact_file_set(tmp_path: Path) -> None:
    from build_app import write_release_manifest

    release_dir = tmp_path / "release"
    (release_dir / "nested").mkdir(parents=True)
    exe = release_dir / "Sky-Auto-Player.exe"
    exe.write_bytes(b"exe")
    (release_dir / "nested" / "data.bin").write_bytes(b"data")
    (release_dir / "nested" / "MANIFEST.json").write_text("payload", encoding="utf-8")
    (release_dir / "_smoke_test.log").write_text("temporary", encoding="utf-8")

    write_release_manifest(release_dir, "2.4.4", exe.name, "deadbeef")

    manifest = json.loads((release_dir / "MANIFEST.json").read_text(encoding="utf-8"))
    assert "executable_sha256" not in manifest
    assert manifest["schema_version"] == 2
    assert manifest["git_head"] == "deadbeef"
    assert manifest["dirty_worktree"] is False
    assert manifest["native_build_commit"] == "deadbeef"
    assert {entry["path"] for entry in manifest["files"]} == {
        "Sky-Auto-Player.exe",
        "nested/MANIFEST.json",
        "nested/data.bin",
    }
    assert not (release_dir / "_smoke_test.log").exists()


def test_write_release_manifest_fails_closed_on_hash_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import build_app

    release_dir = tmp_path / "release"
    release_dir.mkdir()
    exe = release_dir / "Sky-Auto-Player.exe"
    exe.write_bytes(b"exe")
    bad = release_dir / "bad.bin"
    bad.write_bytes(b"bad")
    original_hash = build_app._hash_file

    def fail_bad_file(path: Path) -> str:
        if path == bad:
            raise OSError("simulated read failure")
        return original_hash(path)

    monkeypatch.setattr(build_app, "_hash_file", fail_bad_file)

    with pytest.raises(RuntimeError, match="Failed to hash release asset"):
        build_app.write_release_manifest(release_dir, "2.4.4", exe.name, "deadbeef")


def test_get_git_head_rejects_dirty_release_checkout(monkeypatch: pytest.MonkeyPatch) -> None:
    import build_app

    def fake_run(args, **_kwargs):
        if args[:2] == ["git", "rev-parse"]:
            return SimpleNamespace(returncode=0, stdout="abc123\n")
        return SimpleNamespace(returncode=0, stdout=" M source.rs\n")

    monkeypatch.setattr(build_app.subprocess, "run", fake_run)
    with pytest.raises(RuntimeError, match="clean Git worktree"):
        build_app.get_git_head()
    assert build_app.get_git_head(require_clean=False) == "abc123-dirty"


def test_native_build_commands_use_pinned_toolchain_and_locked_dependencies() -> None:
    import build_app

    assert build_app.get_pinned_rust_toolchain() == "1.98.0"
    assert build_app.native_build_environment()["RUSTUP_TOOLCHAIN"] == "1.98.0"
    for command in (
        build_app.cargo_release_build_command(Path("calibration/Cargo.toml"), "native_calibration"),
        build_app.cargo_release_build_command(Path("updater/Cargo.toml"), "sky_updater"),
    ):
        assert command[-1] == "--locked"


def test_verify_native_build_info_requires_matching_release_metadata(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import build_app

    expected_commit = "a" * 40
    native_info = {
        "native_build_commit": expected_commit,
        "rustc_version": "rustc 1.98.0 (test)",
        "native_abi": build_app.EXPECTED_NATIVE_ABI,
        "schema_version": build_app.RUST_DISPATCH_SCHEMA_VERSION,
        "native_schema_version": build_app.RUST_DISPATCH_SCHEMA_VERSION,
        "free_threaded": True,
        "win32_backend": True,
    }
    fake_native = SimpleNamespace(build_info=lambda: native_info)
    monkeypatch.setitem(sys.modules, "sky_player_rs", fake_native)

    build_app.verify_native_build_info(expected_commit)

    native_info["native_build_commit"] = "b" * 40
    with pytest.raises(RuntimeError, match="native_build_commit"):
        build_app.verify_native_build_info(expected_commit)

    native_info["native_build_commit"] = expected_commit
    native_info["rustc_version"] = "rustc 1.97.1 (wrong-toolchain)"
    with pytest.raises(RuntimeError, match="rustc_version"):
        build_app.verify_native_build_info(expected_commit)

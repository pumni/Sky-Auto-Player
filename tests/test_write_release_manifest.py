import json
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).parents[1]


def _common():
    scripts = str(ROOT / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    import release_common

    return release_common


def test_write_release_manifest_covers_exact_file_set(tmp_path: Path) -> None:
    common = _common()
    release_dir = tmp_path / "release"
    (release_dir / "nested").mkdir(parents=True)
    exe = release_dir / "Sky-Auto-Player.exe"
    exe.write_bytes(b"exe")
    (release_dir / "nested" / "data.bin").write_bytes(b"data")
    (release_dir / "nested" / "MANIFEST.json").write_text("payload", encoding="utf-8")
    (release_dir / "_smoke_test.log").write_text("temporary", encoding="utf-8")

    common.write_release_manifest(release_dir, "2.4.4", exe.name, "deadbeef")

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
    common = _common()
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    exe = release_dir / "Sky-Auto-Player.exe"
    exe.write_bytes(b"exe")
    bad = release_dir / "bad.bin"
    bad.write_bytes(b"bad")
    original_hash = common.hash_file

    def fail_bad_file(path: Path) -> str:
        if path == bad:
            raise OSError("simulated read failure")
        return original_hash(path)

    monkeypatch.setattr(common, "hash_file", fail_bad_file)

    with pytest.raises(RuntimeError, match="Failed to hash release asset"):
        common.write_release_manifest(release_dir, "2.4.4", exe.name, "deadbeef")


def test_get_git_head_rejects_dirty_release_checkout(monkeypatch: pytest.MonkeyPatch) -> None:
    common = _common()

    def fake_run(args, **_kwargs):
        if args[:2] == ["git", "rev-parse"]:
            return SimpleNamespace(returncode=0, stdout="abc123\\n")
        return SimpleNamespace(returncode=0, stdout=" M source.rs\\n")

    monkeypatch.setattr(common.subprocess, "run", fake_run)
    with pytest.raises(RuntimeError, match="clean Git worktree"):
        common.get_git_head()
    assert common.get_git_head(require_clean=False) == "abc123-dirty"


def test_native_build_commands_use_pinned_toolchain_and_locked_dependencies() -> None:
    common = _common()

    assert common.get_pinned_rust_toolchain() == "1.98.0"
    assert common.native_build_environment()["RUSTUP_TOOLCHAIN"] == "1.98.0"
    for command in (
        common.cargo_release_build_command(Path("calibration/Cargo.toml"), "native_calibration"),
        common.cargo_release_build_command(Path("updater/Cargo.toml"), "sky_updater"),
    ):
        assert command[-1] == "--locked"


def test_native_provenance_rejects_mismatched_build_commit() -> None:
    common = _common()
    expected = "a" * 40
    with pytest.raises(RuntimeError, match="does not match repository HEAD"):
        common.validate_native_build_provenance(expected, "b" * 40)
    common.validate_native_build_provenance(expected, expected)

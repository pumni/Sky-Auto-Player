import hashlib
import json
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
    assert manifest["executable_sha256"] == hashlib.sha256(b"exe").hexdigest()
    assert manifest["git_head"] == "deadbeef"
    assert manifest["dirty_worktree"] is False
    assert manifest["native_build_commit"] == "deadbeef"
    assert {entry["path"] for entry in manifest["files"]} == {
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

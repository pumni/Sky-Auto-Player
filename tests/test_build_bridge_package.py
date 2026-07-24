import hashlib
import json
from pathlib import Path

import pytest


def test_build_bridge_package(tmp_path: Path) -> None:
    # Phase 0: Stub test that will fail until the function is implemented in build_app.py
    
    # Create fake canonical dir
    canonical_dir = tmp_path / "Sky-Auto-Player-v2.4.2"
    canonical_dir.mkdir()
    
    # Add Sky-Auto-Player.exe
    exe_content = b"fake-exe-bytes"
    (canonical_dir / "Sky-Auto-Player.exe").write_bytes(exe_content)
    
    # Add dummy files
    (canonical_dir / "foo.txt").write_text("bar")
    
    # In Phase 2, we will implement this function:
    # from build_app import build_legacy_bridge_dir
    # bridge_dir = build_legacy_bridge_dir(canonical_dir, "2.4.2")
    
    from build_app import build_legacy_bridge_dir
    
    bridge_dir = build_legacy_bridge_dir(canonical_dir, "2.4.2")
    
    assert bridge_dir.exists()
    assert (bridge_dir / "Sky-Auto-Player.exe").exists()
    assert (bridge_dir / "Sky-Player.exe").exists()
    assert (bridge_dir / "Sky-Player.exe").read_bytes() == exe_content
    
    manifest = json.loads((bridge_dir / "MANIFEST.json").read_text(encoding="utf-8"))
    assert manifest["version"] == "2.4.2"
    assert manifest["executable"] == "Sky-Player.exe"
    assert manifest["executable_sha256"] == hashlib.sha256(exe_content).hexdigest()
    assert {entry["path"] for entry in manifest["files"]} == {
        "Sky-Auto-Player.exe",
        "foo.txt",
    }


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

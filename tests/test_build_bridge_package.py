import json
from pathlib import Path


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

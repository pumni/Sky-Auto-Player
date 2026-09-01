from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).parents[1]


def _load_phase8():
    path = ROOT / "scripts" / "build_portable_release.py"
    spec = importlib.util.spec_from_file_location("build_phase8_under_test", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load portable release builder")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _load_verifier():
    path = ROOT / "scripts" / "verify_release_manifest.py"
    spec = importlib.util.spec_from_file_location("verify_release_manifest_phase8", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load release verifier")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _seed_release(tmp_path: Path) -> Path:
    release = tmp_path / "release"
    for name, payload in (
        ("Sky-Auto-Player.exe", b"tauri"),
        ("Sky-Auto-Player-Updater.exe", b"updater"),
        ("native_calibration.exe", b"calibration"),
        ("songs/example.json", b"{}"),
    ):
        path = release / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
    from build_app import write_release_manifest

    write_release_manifest(release, "3.5.0", "Sky-Auto-Player.exe", "a" * 40)
    return release


def test_phase8_manifest_rejects_runtime_python_artifacts(tmp_path: Path) -> None:
    verifier = _load_verifier()
    release = _seed_release(tmp_path)
    verifier.verify(release, "3.5.0")

    (release / "Sky-Auto-Player-Core.exe").write_bytes(b"retired core")
    with pytest.raises(RuntimeError, match="bundled Python runtime artifacts"):
        verifier.verify(release, "3.5.0")


def test_phase8_zip_is_byte_deterministic_and_has_sidecar(tmp_path: Path) -> None:
    phase8 = _load_phase8()
    release = _seed_release(tmp_path)
    first, first_hash = phase8.write_deterministic_zip(release, tmp_path / "one.zip")
    second, second_hash = phase8.write_deterministic_zip(release, tmp_path / "two.zip")
    assert first.read_bytes() == second.read_bytes()
    assert first_hash == second_hash == hashlib.sha256(first.read_bytes()).hexdigest()
    assert (tmp_path / "one.zip.sha256").read_text(encoding="ascii") == (
        f"{first_hash}  one.zip\n"
    )


def test_phase8_provenance_has_public_head_and_exact_artifact_hash(tmp_path: Path, monkeypatch) -> None:
    phase8 = _load_phase8()
    release = _seed_release(tmp_path)
    zip_path, _ = phase8.write_deterministic_zip(release, tmp_path / "artifact.zip")
    monkeypatch.setattr(phase8, "_capture", lambda command: "test-version")
    provenance_path = tmp_path / "PROVENANCE.json"
    data = phase8.write_provenance(
        provenance_path,
        repo_head="a" * 40,
        release_dir=release,
        zip_path=zip_path,
        native_build_commit="a" * 40,
    )
    assert data["repo_head"] == "a" * 40
    assert data["artifact"]["sha256"] == hashlib.sha256(zip_path.read_bytes()).hexdigest()
    assert json.loads(provenance_path.read_text(encoding="utf-8"))["artifact"]["file_count"] == 5


def test_phase8_spec_is_the_single_core_runtime() -> None:
    spec = (ROOT / "Sky-Auto-Player-Core.spec").read_text(encoding="utf-8")
    assert "src\" / \"core_main.py" in spec
    assert 'app_name = "Sky-Auto-Player-Core"' in spec
    assert '"main"' in spec
    assert '"sky_player_rs.sky_player_rs"' in spec


def test_play_batch_prefers_packaged_core_tui() -> None:
    batch = (ROOT / "play.bat").read_text(encoding="utf-8")
    assert "Sky-Auto-Player-Core.exe" in batch
    assert "--tui" in batch

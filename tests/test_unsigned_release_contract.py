from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType

import pytest

ROOT = Path(__file__).parents[1]


def _load_manifest_verifier() -> ModuleType:
    path = ROOT / "scripts" / "verify_release_manifest.py"
    spec = importlib.util.spec_from_file_location("verify_release_manifest", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load release manifest verifier")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_release_workflow_is_unsigned_but_keeps_integrity_gates() -> None:
    workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    for forbidden in (
        "AUTHENTICODE_PFX_BASE64",
        "AUTHENTICODE_PFX_PASSWORD",
        "AUTHENTICODE_CERT_THUMBPRINT",
        "EXPECTED_PUBLISHER_SUBJECT",
        "SKY_PUBLISHER_SUBJECT",
        "Import-PfxCertificate",
        "signtool",
        "verify_release_signatures.ps1",
        "Sign project-owned PE files",
    ):
        assert forbidden not in workflow
    for required in (
        "scripts/audit_security_mandates.py",
        "scripts/build_pyinstaller_bootloader.ps1",
        "scripts/verify_release_manifest.py",
        "actions/attest-build-provenance@",
        "Sky-Auto-Player-v${{ steps.stage.outputs.version }}.zip.sha256",
        "MANIFEST.json",
    ):
        assert required in workflow


def test_unsigned_release_tree_is_exact_and_has_verified_native_updater(tmp_path: Path) -> None:
    verifier = _load_manifest_verifier()
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    (release_dir / "Sky-Auto-Player.exe").write_bytes(b"app")
    (release_dir / "native_calibration.exe").write_bytes(b"calibration")
    (release_dir / "Sky-Auto-Player-Updater.exe").write_bytes(b"updater")
    from build_app import write_release_manifest

    write_release_manifest(release_dir, "3.2.1", "Sky-Auto-Player.exe", "a" * 40)
    verifier.verify(release_dir, "3.2.1")
    (release_dir / "Sky-Auto-Player-Updater.exe").write_bytes(b"tampered")
    with pytest.raises(RuntimeError, match="manifest hash/size mismatch"):
        verifier.verify(release_dir, "3.2.1")


@pytest.mark.parametrize("legacy_name", ["Sky-Player.exe", "updater.bat", "updater.ps1"])
def test_release_tree_rejects_legacy_update_artifacts(tmp_path: Path, legacy_name: str) -> None:
    verifier = _load_manifest_verifier()
    release_dir = tmp_path / "release"
    release_dir.mkdir()
    (release_dir / "Sky-Auto-Player.exe").write_bytes(b"app")
    (release_dir / "native_calibration.exe").write_bytes(b"calibration")
    (release_dir / "Sky-Auto-Player-Updater.exe").write_bytes(b"updater")
    from build_app import write_release_manifest

    write_release_manifest(release_dir, "3.2.1", "Sky-Auto-Player.exe", "a" * 40)
    (release_dir / legacy_name).write_bytes(b"legacy")
    with pytest.raises(RuntimeError, match="forbidden artifacts"):
        verifier.verify(release_dir, "3.2.1")


def test_manual_download_url_is_fixed_to_official_repository() -> None:
    from sky_music.orchestration.update_service import OFFICIAL_RELEASES_URL

    assert OFFICIAL_RELEASES_URL == "https://github.com/pumni/Sky-Auto-Player/releases"


def test_manual_download_action_never_uses_release_metadata_url(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from sky_music.orchestration.update_service import OFFICIAL_RELEASES_URL
    from sky_music.ui.textual_app import app as app_module

    opened: list[tuple[str, int]] = []
    monkeypatch.setattr(
        app_module.webbrowser,
        "open",
        lambda url, new=0: opened.append((url, new)) or True,
    )
    fake_app = type("FakeApp", (), {"notify": lambda *_args, **_kwargs: None})()
    app_module.SkyPickerApp._open_manual_update_page(fake_app)  # type: ignore[arg-type]

    assert opened == [(OFFICIAL_RELEASES_URL, 2)]


def test_runtime_contains_no_signature_bypass_flags() -> None:
    forbidden = ("--skip-signature", "--no-verify", "disable-verification")
    for path in (ROOT / "src").rglob("*.py"):
        source = path.read_text(encoding="utf-8")
        assert not any(flag in source for flag in forbidden), path

"""Tests for ``sky_music.orchestration.update_service``.

The service wires together domain logic, persistence, and the installer. Tests
stub the underlying fetch / installer calls so they exercise the delegation
without touching the network or filesystem.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

import sky_music.config as config_mod
from sky_music.config import AppConfig, UpdateSettings, clear_config_cache
from sky_music.domain.update_checker import (
    UpdateCheckResult,
    UpdateInfo,
    parse_release_payload,
)
from sky_music.orchestration.update_service import (
    apply_staged_update,
    check_for_update,
    current_unix_ts,
    download_and_verify_update,
    record_skip,
    record_successful_check,
    should_auto_check,
)


@pytest.fixture(autouse=True)
def _reset_config_cache():
    clear_config_cache()
    yield
    clear_config_cache()


@pytest.fixture
def isolated_config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    cfg_path = tmp_path / "config.json"
    cfg_path.write_text("{}", encoding="utf-8")
    monkeypatch.setattr(config_mod, "CONFIG_PATH", cfg_path)
    return cfg_path


def _make_release_payload(
    *,
    tag: str = "v2.4.0",
    assets: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    return {
        "tag_name": tag,
        "html_url": "https://github.com/pumni/Sky-Player/releases/tag/" + tag,
        "published_at": "2024-01-01T00:00:00Z",
        "body": "Update notes",
        "assets": assets
        or [
            {
                "name": "Sky-Player-v2.4.0.zip",
                "browser_download_url": "https://example.com/x.zip",
            },
            {
                "name": "Sky-Player-v2.4.0.zip.sha256",
                "browser_download_url": "https://example.com/x.zip.sha256",
            },
        ],
    }


# ── current_unix_ts ───────────────────────────────────────────────────────────


def test_current_unix_ts_is_int(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        "sky_music.orchestration.update_service.time.time", lambda: 1718200000.5
    )
    ts = current_unix_ts()
    assert isinstance(ts, int)
    assert ts == 1718200000


# ── should_auto_check ────────────────────────────────────────────────────────


def test_should_auto_check_default_true_and_first_run(isolated_config: Path) -> None:
    cfg = AppConfig()
    assert should_auto_check(cfg, now_ts=1718200000) is True


def test_should_auto_check_disabled_overrides_throttle(isolated_config: Path) -> None:
    cfg = AppConfig(update=UpdateSettings(auto_check=False))
    assert should_auto_check(cfg, now_ts=99_999_999) is False


def test_should_auto_check_within_throttle_returns_false(
    isolated_config: Path,
) -> None:
    cfg = AppConfig(
        update=UpdateSettings(auto_check=True, check_interval_s=86400, last_check_ts=1000),
    )
    assert should_auto_check(cfg, now_ts=1500) is False


def test_should_auto_check_after_throttle_returns_true(
    isolated_config: Path,
) -> None:
    cfg = AppConfig(
        update=UpdateSettings(auto_check=True, check_interval_s=86400, last_check_ts=1000),
    )
    assert should_auto_check(cfg, now_ts=1000 + 86400) is True


def test_should_auto_check_clock_skew_allows_check(isolated_config: Path) -> None:
    cfg = AppConfig(
        update=UpdateSettings(auto_check=True, check_interval_s=86400, last_check_ts=2000),
    )
    assert should_auto_check(cfg, now_ts=1000) is True


# ── check_for_update ──────────────────────────────────────────────────────────


def test_check_for_update_returns_update_info_with_sha256_url(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = _make_release_payload()
    cfg = AppConfig()

    def fake_fetch(**kwargs: Any) -> UpdateCheckResult:
        return parse_release_payload(
            payload,
            current_version=kwargs["current_version"],
            skip_version=kwargs.get("skip_version"),
        )

    monkeypatch.setattr(
        "sky_music.orchestration.update_service.fetch_latest_release", fake_fetch
    )
    result = check_for_update(cfg, current_version="2.3.0")
    assert result.update is not None
    assert result.update.latest_version == "2.4.0"
    assert result.update.sha256_url == "https://example.com/x.zip.sha256"


def test_check_for_update_skips_version_when_configured(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = _make_release_payload()
    cfg = AppConfig(update=UpdateSettings(skip_version="2.4.0"))

    def fake_fetch(**kwargs: Any) -> UpdateCheckResult:
        return parse_release_payload(
            payload,
            current_version=kwargs["current_version"],
            skip_version=kwargs.get("skip_version"),
        )

    monkeypatch.setattr(
        "sky_music.orchestration.update_service.fetch_latest_release", fake_fetch
    )
    result = check_for_update(cfg, current_version="2.3.0")
    assert result.update is None  # suppressed by skip_version


# ── record_skip / record_successful_check ──────────────────────────────────────


def test_record_skip_persists_to_config(isolated_config: Path) -> None:
    from sky_music.config import load_config

    cfg = AppConfig()
    record_skip(cfg, "2.4.0")

    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.skip_version == "2.4.0"


def test_record_skip_clears_with_empty(isolated_config: Path) -> None:
    from sky_music.config import load_config

    cfg = AppConfig()
    record_skip(cfg, "2.4.0")
    record_skip(cfg, "")

    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.skip_version == ""


def test_record_successful_check_writes_timestamp(isolated_config: Path) -> None:
    from sky_music.config import load_config

    cfg = AppConfig()
    record_successful_check(cfg, now_ts=1718200000)

    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.last_check_ts == 1718200000


# ── download_and_verify_update ───────────────────────────────────────────────


def _patch_opener(monkeypatch: pytest.MonkeyPatch, body: bytes) -> None:
    """Replace the installer's default opener so stage_update uses our body."""
    import io

    import sky_music.infrastructure.update_installer as installer_mod

    class _Resp:
        def __init__(self, data: bytes) -> None:
            self._buf = io.BytesIO(data)
            self.headers: dict[str, str] = {
                "Content-Length": str(len(data)),
            }

        def __enter__(self) -> _Resp:
            return self

        def __exit__(self, *_: object) -> None:
            return None

        def read(self, size: int = -1) -> bytes:
            return self._buf.read(-1 if size == -1 else size)

    def _opener(url: str, *, timeout: float = 0.0) -> _Resp:
        _ = url, timeout
        return _Resp(body)

    monkeypatch.setattr(installer_mod, "_urlopen_default", _opener)


def test_download_and_verify_update_missing_asset_returns_error(
    isolated_config: Path,
) -> None:
    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="",
    )
    outcome = download_and_verify_update(release)
    assert outcome.staged is None
    assert outcome.error == "release has no download asset"


def test_download_and_verify_update_no_sidecar_stages_anyway(
    isolated_config: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import io
    import zipfile

    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("notes.txt", b"hello")
    body = buf.getvalue()

    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="https://example.com/x.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="",  # no sidecar
    )
    staging_parent = tmp_path / "staging"
    _patch_opener(monkeypatch, body)

    outcome = download_and_verify_update(release, staging_parent=staging_parent)
    assert outcome.error is None, f"unexpected error: {outcome.error}"
    assert outcome.staged is not None
    assert outcome.staged.new_version == "2.4.0"
    assert (outcome.staged.staging_dir / "notes.txt").read_bytes() == b"hello"


def test_download_and_verify_update_with_sha256_match_succeeds(
    isolated_config: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import hashlib
    import io
    import zipfile

    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("notes.txt", b"hello")
    body = buf.getvalue()
    expected_sha = hashlib.sha256(body).hexdigest()

    # Sidecar response: provide a bare hash on first line.
    class _Resp:
        def __init__(self, data: bytes, headers: dict[str, str] | None = None) -> None:
            self._buf = io.BytesIO(data)
            self.headers = headers or {}

        def __enter__(self) -> _Resp:
            return self

        def __exit__(self, *_: object) -> None:
            return None

        def read(self, size: int = -1) -> bytes:
            return self._buf.read(-1 if size == -1 else size)

    # Routing: zipfile URL → zip bytes; .sha256 URL → sidecar text.
    def _opener(url: str, *, timeout: float = 0.0) -> _Resp:
        _ = timeout
        if url.endswith(".sha256"):
            return _Resp(expected_sha.encode("utf-8"), {})
        return _Resp(body, {"Content-Length": str(len(body))})

    import sky_music.infrastructure.update_installer as installer_mod

    monkeypatch.setattr(installer_mod, "_urlopen_default", _opener)

    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="https://example.com/x.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="https://example.com/x.zip.sha256",
    )
    outcome = download_and_verify_update(
        release, staging_parent=tmp_path / "staging"
    )
    assert outcome.error is None
    assert outcome.staged is not None


def test_download_and_verify_update_sha256_mismatch_returns_error(
    isolated_config: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import io
    import zipfile

    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("notes.txt", b"hello")
    body = buf.getvalue()
    bad_sha = "0" * 64

    class _Resp:
        def __init__(self, data: bytes, headers: dict[str, str] | None = None) -> None:
            self._buf = io.BytesIO(data)
            self.headers = headers or {}

        def __enter__(self) -> _Resp:
            return self

        def __exit__(self, *_: object) -> None:
            return None

        def read(self, size: int = -1) -> bytes:
            return self._buf.read(-1 if size == -1 else size)

    def _opener(url: str, *, timeout: float = 0.0) -> _Resp:
        _ = timeout
        if url.endswith(".sha256"):
            return _Resp(bad_sha.encode("utf-8"), {})
        return _Resp(body, {"Content-Length": str(len(body))})

    import sky_music.infrastructure.update_installer as installer_mod

    monkeypatch.setattr(installer_mod, "_urlopen_default", _opener)

    release = UpdateInfo(
        latest_version="2.4.0",
        download_url="https://example.com/x.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url="https://example.com/x.zip.sha256",
    )
    outcome = download_and_verify_update(
        release, staging_parent=tmp_path / "staging"
    )
    assert outcome.staged is None
    assert outcome.error is not None
    assert "sha256 mismatch" in outcome.error


# ── apply_staged_update ───────────────────────────────────────────────────────


def test_apply_staged_update_non_windows_platform_raises(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import sys

    from sky_music.infrastructure import update_installer as installer_mod
    from sky_music.infrastructure.update_installer import StagedUpdate
    from sky_music.orchestration import update_service as service_mod

    # Patch sys.platform on the shared ``sys`` module — both the installer and
    # the service read ``sys.platform`` from the same module object, so a
    # single patch works.
    monkeypatch.setattr(sys, "platform", "linux")

    install_dir = tmp_path_for_apply() / "install"
    install_dir.mkdir(parents=True, exist_ok=True)
    staged = StagedUpdate(
        staging_dir=tmp_path_for_apply() / "staging", new_version="2.4.0"
    )
    with pytest.raises(Exception, match="Windows-only"):
        apply_staged_update(staged, install_dir=install_dir)
    # Touch installer and service modules to silence unused-import lint.
    _ = installer_mod, service_mod


def tmp_path_for_apply() -> Path:
    """Return a unique dir under the OS temp root so test isolation holds.

    A fixture cannot be invoked from a helper used in a parametrize; we fall
    back to creating a per-call temp subdir. Tests using this should clean up
    in a try/finally — but apply_staged_update with a patched sys.platform
    never actually touches the filesystem, so cleanup isn't critical.
    """
    import tempfile
    import uuid

    base = Path(tempfile.gettempdir()) / "sky-update-test" / uuid.uuid4().hex
    base.mkdir(parents=True, exist_ok=True)
    return base

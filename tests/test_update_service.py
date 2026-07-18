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
    parse_release_payload,
)
from sky_music.orchestration.update_service import (
    check_for_update,
    current_unix_ts,
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


def test_should_auto_check_retry_gate_blocks_within_window(
    isolated_config: Path,
) -> None:
    """Recent failed fetch + still within the 5-minute backoff window → blocked.

    Establishes the fix for Bug F: a one-off network blip used to lock the
    user out of update notifications until the long ``check_interval_s``
    elapsed (24h default). The new short-backoff gate retries every 5 min.
    """
    cfg = AppConfig(
        update=UpdateSettings(
            auto_check=True,
            check_interval_s=86400,
            last_check_ts=0,
            last_error_ts=1000,
        ),
    )
    # gap=60 < 300 → no retry yet
    assert should_auto_check(cfg, now_ts=1000 + 60) is False
    # gap=299 < 300 → still blocked
    assert should_auto_check(cfg, now_ts=1000 + 299) is False


def test_should_auto_check_retry_gate_allows_at_window_boundary(
    isolated_config: Path,
) -> None:
    """At gap == _RETRY_INTERVAL_S (5 min) the backoff gate opens."""
    cfg = AppConfig(
        update=UpdateSettings(
            auto_check=True,
            check_interval_s=86400,
            last_check_ts=0,
            last_error_ts=1000,
        ),
    )
    # gap=300 == window edge → allowed
    assert should_auto_check(cfg, now_ts=1000 + 300) is True
    # gap=500 → past window → allowed
    assert should_auto_check(cfg, now_ts=1000 + 500) is True


def test_should_auto_check_disabled_overrides_retry_gate(isolated_config: Path) -> None:
    """auto_check=False must short-circuit even when a backoff retry is due."""
    cfg = AppConfig(
        update=UpdateSettings(
            auto_check=False,
            last_error_ts=1000,
        ),
    )
    assert should_auto_check(cfg, now_ts=1000 + 10_000) is False


def test_should_auto_check_retry_gate_clock_skew_allows(isolated_config: Path) -> None:
    """Negative gap (clock skew backwards) lets the retry gate fire."""
    cfg = AppConfig(
        update=UpdateSettings(
            auto_check=True,
            last_error_ts=2000,
        ),
    )
    assert should_auto_check(cfg, now_ts=1000) is True


def test_retry_delay_for_zero_when_no_error(isolated_config: Path) -> None:
    from sky_music.orchestration.update_service import retry_delay_for

    cfg = AppConfig()
    assert retry_delay_for(cfg, now_ts=1000) == 0


def test_retry_delay_for_returns_seconds_until_window_end(isolated_config: Path) -> None:
    from sky_music.orchestration.update_service import retry_delay_for

    cfg = AppConfig(
        update=UpdateSettings(last_error_ts=1000, last_check_ts=0),
    )
    assert retry_delay_for(cfg, now_ts=1000 + 100) == 200  # 300 - 100
    assert retry_delay_for(cfg, now_ts=1000 + 300) == 0  # right at boundary
    assert retry_delay_for(cfg, now_ts=1000 + 500) == 0  # past boundary


def test_record_check_error_persists_last_error_ts(isolated_config: Path) -> None:
    from sky_music.config import load_config
    from sky_music.orchestration.update_service import record_check_error

    cfg = AppConfig()
    record_check_error(cfg, now_ts=12345)

    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.last_error_ts == 12345


def test_record_successful_check_clears_last_error_ts(isolated_config: Path) -> None:
    """A successful check must reset last_error_ts so the backoff gate stops."""
    from sky_music.config import load_config
    from sky_music.orchestration.update_service import (
        record_check_error,
        record_successful_check,
    )

    cfg = AppConfig()
    record_check_error(cfg, now_ts=1000)
    assert cfg.update.last_error_ts == 1000

    record_successful_check(cfg, now_ts=5000)
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.last_error_ts == 0
    assert reloaded.update.last_check_ts == 5000


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

def test_check_for_update_beta_channel_passes_channel(
    isolated_config: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from sky_music.config import AppConfig, UpdateSettings
    cfg = AppConfig(update=UpdateSettings(channel="beta"))
    
    called_with_channel = None

    def fake_fetch(**kwargs: Any) -> Any:
        nonlocal called_with_channel
        called_with_channel = kwargs.get("channel")
        from sky_music.domain.update_checker import UpdateCheckResult
        return UpdateCheckResult(update=None, current_version="2.3.0")

    monkeypatch.setattr(
        "sky_music.orchestration.update_service.fetch_latest_release", fake_fetch
    )
    from sky_music.orchestration.update_service import check_for_update
    check_for_update(cfg, current_version="2.3.0")
    assert called_with_channel == "beta"


def test_format_update_banner_no_notes() -> None:
    from sky_music.domain.update_checker import UpdateInfo
    from sky_music.orchestration.update_service import format_update_banner
    update = UpdateInfo(
        latest_version="2.0.1",
        download_url="url",
        release_notes="",
        html_url="html",
        published_at="time"
    )
    banner = format_update_banner(update, current_version="2.0.0")
    assert "Sky Player v2.0.1 is now available." in banner
    assert "You are running v2.0.0." in banner
    assert "(no release notes)" in banner

def test_format_update_banner_truncates_long_notes() -> None:
    from sky_music.domain.update_checker import UpdateInfo
    from sky_music.orchestration.update_service import format_update_banner
    notes = "\n".join(f"line {i}" for i in range(15))
    update = UpdateInfo(
        latest_version="2.0.1",
        download_url="url",
        release_notes=notes,
        html_url="html",
        published_at="time"
    )
    banner = format_update_banner(update, current_version="2.0.0")
    assert "line 9" in banner
    assert "line 10" not in banner
    assert "... (see GitHub for full notes)" in banner

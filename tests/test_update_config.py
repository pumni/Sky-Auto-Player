"""Tests for the update-related persistence additions in ``sky_music.config``."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

import sky_music.config as config_mod
from sky_music.config import (
    AppConfig,
    UpdateSettings,
    clear_config_cache,
)


@pytest.fixture(autouse=True)
def _reset_config_cache():
    clear_config_cache()
    yield
    clear_config_cache()


@pytest.fixture
def isolated_config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Redirect ``CONFIG_PATH`` to an isolated tmp file for save_config tests."""
    cfg_path = tmp_path / "config.json"
    cfg_path.write_text("{}", encoding="utf-8")
    monkeypatch.setattr(config_mod, "CONFIG_PATH", cfg_path)
    return cfg_path


# ── UpdateSettings.from_dict ──────────────────────────────────────────────────


def test_update_settings_defaults() -> None:
    s = UpdateSettings()
    assert s.auto_check is True
    assert s.channel == "stable"
    assert s.skip_version == ""
    assert s.check_interval_s == 86400
    assert s.last_check_ts == 0
    assert s.last_notified_version == ""
    assert s.legacy_old_dir_sweep_pending is False


def test_update_settings_from_dict_full() -> None:
    raw: dict[str, Any] = {
        "auto_check": False,
        "channel": "beta",
        "skip_version": "2.4.0",
        "check_interval_s": 3600,
        "last_check_ts": 1718200000,
        "last_notified_version": "2.4.0",
        "legacy_old_dir_sweep_pending": True,
    }
    s = UpdateSettings.from_dict(raw)
    assert s.auto_check is False
    assert s.channel == "beta"
    assert s.skip_version == "2.4.0"
    assert s.check_interval_s == 3600
    assert s.last_check_ts == 1718200000
    assert s.last_notified_version == "2.4.0"
    assert s.legacy_old_dir_sweep_pending is True


def test_update_settings_from_dict_optional_keys_use_defaults() -> None:
    s = UpdateSettings.from_dict({"auto_check": False})
    assert s.auto_check is False
    assert s.channel == "stable"
    assert s.skip_version == ""
    assert s.check_interval_s == 86400
    assert s.last_check_ts == 0
    assert s.last_notified_version == ""
    assert s.legacy_old_dir_sweep_pending is False


def test_update_settings_from_dict_non_dict_returns_defaults() -> None:
    s = UpdateSettings.from_dict("not a dict")  # type: ignore[arg-type]
    assert s.auto_check is True  # default
    assert s.channel == "stable"
    assert s.check_interval_s == 86400


def test_update_settings_from_dict_invalid_integers_clamp() -> None:
    raw = {"check_interval_s": "garbage", "last_check_ts": "n/a"}
    s = UpdateSettings.from_dict(raw)
    assert s.check_interval_s == 86400  # falls back to default
    assert s.last_check_ts == 0


def test_update_settings_from_dict_negative_interval_clamps_to_zero() -> None:
    s = UpdateSettings.from_dict({"check_interval_s": -10})
    assert s.check_interval_s == 0


def test_update_settings_from_dict_skip_version_normalizes_none_to_empty() -> None:
    s = UpdateSettings.from_dict({"skip_version": None})
    assert s.skip_version == ""


def test_update_settings_from_dict_channel_invalid_fallback() -> None:
    s = UpdateSettings.from_dict({"channel": "invalid"})
    assert s.channel == "stable"


def test_update_settings_from_dict_migration_trigger() -> None:
    s1 = UpdateSettings.from_dict({"pending_update_version": "2.0"})
    assert s1.legacy_old_dir_sweep_pending is True
    s2 = UpdateSettings.from_dict({"auto_apply": True})
    assert s2.legacy_old_dir_sweep_pending is True
    s3 = UpdateSettings.from_dict({"auto_apply": False})
    assert s3.legacy_old_dir_sweep_pending is True


# ── AppConfig.update field ─────────────────────────────────────────────────────


def test_appconfig_default_includes_update_settings() -> None:
    cfg = AppConfig()
    assert isinstance(cfg.update, UpdateSettings)
    assert cfg.update.auto_check is True


def test_appconfig_factory_produces_independent_instances() -> None:
    a = AppConfig()
    b = AppConfig()
    a.update.skip_version = "x"
    assert b.update.skip_version == ""


# ── load_config / save_config round-trip ──────────────────────────────────────


def test_save_config_persists_update_block(isolated_config: Path) -> None:
    from sky_music.config import save_config

    cfg = AppConfig()
    cfg.update.auto_check = False
    cfg.update.skip_version = "2.5.0"
    cfg.update.check_interval_s = 7200
    cfg.update.last_check_ts = 1718200000

    save_config(cfg)
    text = isolated_config.read_text(encoding="utf-8")
    raw = json.loads(text)
    assert "update" in raw
    assert raw["update"]["auto_check"] is False
    assert raw["update"]["skip_version"] == "2.5.0"
    assert raw["update"]["check_interval_s"] == 7200
    assert raw["update"]["last_check_ts"] == 1718200000


def test_load_config_round_trips_update_block(isolated_config: Path) -> None:
    from sky_music.config import load_config, save_config

    cfg = AppConfig()
    cfg.update.auto_check = False
    cfg.update.skip_version = "2.5.0"
    save_config(cfg)

    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.auto_check is False
    assert reloaded.update.skip_version == "2.5.0"


def test_load_config_handles_missing_update_block(isolated_config: Path) -> None:
    isolated_config.write_text(
        json.dumps({"theme": "aurora", "schema_version": 2}),
        encoding="utf-8",
    )
    from sky_music.config import load_config

    cfg = load_config(force_reload=True)
    assert cfg.update == UpdateSettings()  # defaults


def test_load_config_handles_malformed_update_block(isolated_config: Path) -> None:
    isolated_config.write_text(
        json.dumps({"update": "not a dict"}),
        encoding="utf-8",
    )
    from sky_music.config import load_config

    cfg = load_config(force_reload=True)
    # Malformed update block falls back to defaults — never raises.
    assert cfg.update == UpdateSettings()


# ── persistence helpers ───────────────────────────────────────────────────────


def test_persist_update_skip_version_writes_string(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_skip_version,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_skip_version(cfg, "2.4.0")

    # Reload and verify
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.skip_version == "2.4.0"


def test_persist_update_skip_version_clears_with_empty(
    isolated_config: Path,
) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_skip_version,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_skip_version(cfg, "2.4.0")
    persist_update_skip_version(cfg, "")
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.skip_version == ""


def test_persist_update_check_ts(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_check_ts,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_check_ts(cfg, 12345)
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.last_check_ts == 12345


def test_persist_update_auto_check(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_auto_check,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_auto_check(cfg, False)
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.auto_check is False


def test_persist_update_channel(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_channel,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_channel(cfg, "beta")
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.channel == "beta"


def test_persist_update_last_notified(isolated_config: Path) -> None:
    from sky_music.config import (
        clear_config_cache,
        load_config,
        persist_update_last_notified,
    )

    clear_config_cache()
    cfg = load_config(force_reload=True)
    persist_update_last_notified(cfg, "2.4.0")
    clear_config_cache()
    reloaded = load_config(force_reload=True)
    assert reloaded.update.last_notified_version == "2.4.0"


# ── save_config concurrency (Bug D regression guard) ──────────────────────────
#
# Bug D: the original save_config held _runtime_cfg_lock only around the
# in-memory cache update, NOT around the file write. Two threads doing
# save_config concurrently could each truncate CONFIG_PATH.open("w") and emit
# interleaved JSON, corrupting config.json. The fix wraps the full RMW in
# _runtime_cfg_lock and uses os.replace(tmp, path) so a concurrent reader
# either sees the old or new contents atomically — never a partial write.
#
# The test races N threads doing save_config with distinct field values
# (different theme + auto_check + skip_version combos). After joining, the
# config file must: (a) parse as valid JSON, (b) carry the schema_version
# field, (c) carry consistent values from the *last* winning writer (one
# tarefa's full overlay — fields must not bleed across writers).
#
# Note: this is a regression test on the LOCK + atomic-replace strategy,
# not on absence of *content* races (a write that loads-then-overlays will
# still lose updates made between load and write by another writer);
# seemingly "dropped" updates are the intended consequence of last-writer-
# wins for a single user-facing setting change, but a TRUNCATED OR
# INTERLEAVED file is a corruption bug and is what this test guards.

def test_save_config_concurrent_writes_do_not_corrupt(
    isolated_config: Path,
) -> None:
    import threading

    from sky_music.config import save_config

    N_THREADS = 8
    N_ITERS = 25
    barrier = threading.Barrier(N_THREADS)
    errors: list[BaseException] = []

    def worker(idx: int) -> None:
        try:
            barrier.wait()
            for i in range(N_ITERS):
                cfg = AppConfig()
                cfg.theme = f"theme-{idx}-{i}"
                cfg.update.auto_check = (i % 2 == 0)
                cfg.update.skip_version = f"sk-{idx}-{i}"
                save_config(cfg)
        except BaseException as e:
            errors.append(e)

    threads = [threading.Thread(target=worker, args=(i,), daemon=True) for i in range(N_THREADS)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=30)

    assert errors == [], f"worker exceptions: {errors}"

    # File must parse cleanly — no truncated JSON or interleaved writes.
    raw = json.loads(isolated_config.read_text(encoding="utf-8"))

    # Every save_config writes schema_version — verifies the file is one
    # writer's complete output, not a partial overlay.
    assert raw["schema_version"] == config_mod.SCHEMA_VERSION

    # The last writer must have written all three of *its* fields — a
    # truncated/partial write would surface as mismatched index suffixes
    # (e.g. theme from writer A but skip_version from writer B).
    theme = raw["theme"]
    auto_check = raw["update"]["auto_check"]
    skip_version = raw["update"]["skip_version"]
    # All three fields share the same "{idx}-{i}" suffix pair — i.e. they
    # were emitted by the same save_config call. The exact (idx, i) pair
    # doesn't matter; structural consistency does.
    assert theme.startswith("theme-")
    assert skip_version.startswith("sk-")
    theme_suffix = theme[len("theme-"):]
    skip_suffix = skip_version[len("sk-"):]
    assert theme_suffix == skip_suffix, (
        f"interleaved write detected: theme={theme_suffix} but "
        f"skip_version={skip_suffix} (Bug D regression — partial write)"
    )
    # auto_check is a bool — its value should be (i % 2 == 0) for whatever
    # i the last writer used. We can't easily verify the bool directly
    # without re-deriving i, but we can at least assert it is a proper bool
    # (not a stray int from a half-written JSON update).
    assert isinstance(auto_check, bool)


def test_save_config_is_atomic_under_reader(isolated_config: Path) -> None:
    """A reader that opens CONFIG_PATH mid-write must never see a truncated body.

    Bug D regression guard part 2: BEFORE the fix, the writer used
    ``CONFIG_PATH.open("w")`` which truncates the file *first*, then writes
    the JSON body. A reader opening the file in that window saw ``{`` or
    ``""`` and would throw ``json.JSONDecodeError``. AFTER the fix, the
    writer writes to ``config.json.tmp`` and ``os.replace``-swaps it in —
    so a reader always observes either the old or the new contents, never
    a partial file.
    """
    import threading

    from sky_music.config import save_config

    stop = threading.Event()
    reader_errors: list[BaseException] = []

    def reader() -> None:
        while not stop.is_set():
            try:
                # The reader does NOT hold _runtime_cfg_lock — real readers
                # (load_config at startup, the updater.ps1 patch step) do not
                # take it either, so this mirrors a real concurrent observer.
                text = isolated_config.read_text(encoding="utf-8")
                json.loads(text)
            except json.JSONDecodeError as e:
                # JSON corruption is the Bug D failure mode — surface it.
                reader_errors.append(e)
            except OSError:
                # Windows: during os.replace, a brief window exists where the
                # target file handle returns ERROR_SHARING_VIOLATION /
                # PermissionError to other openers. This is the kernel's
                # file-lock semantics for a swap-in-progress — not corruption.
                # The next iteration will see either the old or new complete
                # contents (atomic guarantee). Swallowing OSError here is
                # therefore correct and matches what load_config already does
                # (returns {} via _load_raw's broad except).
                pass

    def writer() -> None:
        for i in range(200):
            cfg = AppConfig()
            cfg.theme = f"theme-{i}"
            cfg.update.skip_version = f"sk-{i}"
            save_config(cfg)

    reader_thread = threading.Thread(target=reader, daemon=True)
    reader_thread.start()
    writer()
    stop.set()
    reader_thread.join(timeout=5)
    assert not reader_errors, (
        f"reader observed truncated/corrupt config.json: {reader_errors[:3]}"
    )

"""Update notifier orchestrator.

Glue layer that ties together the pure version-check domain logic
(:mod:`sky_music.domain.update_checker`), the persistence layer
(:mod:`sky_music.config` for skip-version / last-check timestamps), and the
Windows-side installer (``updater.ps1``).

The UI only needs to:
  1. Call :func:`should_auto_check` before launching the background check.
  2. Call :func:`check_for_update` in a background thread / worker.
  3. On completion, inspect the returned :class:`UpdateCheckResult`; if there
     is a newer version the user has not skipped, surface it via
     :class:`sky_music.ui.textual_app.modals.UpdateModal`.

This module never blocks the dispatcher or UI thread; callers run network
operations in their own worker.
"""

from __future__ import annotations

import time

from sky_music.config import (
    AppConfig,
    persist_update_check_ts,
    persist_update_error_ts,
    persist_update_skip_version,
)
from sky_music.domain.update_checker import (
    UpdateCheckResult,
    UpdateInfo,
    fetch_latest_release,
)


def format_update_banner(update: UpdateInfo, current_version: str) -> str:
    """Format the update banner string."""
    latest = update.latest_version
    
    lines = [
        f"Sky Player v{latest} is now available.",
        f"You are running v{current_version}.",
        "To update: close Sky Player, run updater.bat, reopen.",
        ""
    ]
    
    notes = (update.release_notes or "").strip()
    if notes:
        # Truncate to max 10 lines
        note_lines = notes.splitlines()
        if len(note_lines) > 10:
            note_lines = note_lines[:10]
            note_lines.append("... (see GitHub for full notes)")
        lines.append("\n".join(note_lines))
    else:
        lines.append("(no release notes)")
        
    return "\n".join(lines)


def current_unix_ts() -> int:
    """Return ``int(time.time())`` — isolated for testability."""
    return int(time.time())


# Short-static retry interval, applied independently of the long success
# throttle ``check_interval_s``. Each time an auto-check fails the
# ``last_error_ts`` is recorded, so this delay applies *per attempt*: a
# failing link retries every 5 minutes instead of being locked out for a
# full ``check_interval_s`` day. We do NOT use an accumulating ladder because
# ``last_error_ts`` is rewritten on every failure, resetting the window — a
# 5-minute constant is the modern Sparkle/Squirrel default.
_RETRY_INTERVAL_S: int = 300  # 5 minutes


def should_auto_check(cfg: AppConfig, *, now_ts: int | None = None) -> bool:
    """True iff automatic update check should fire right now.

    Two gates, OR'd together, behind the global ``auto_check`` toggle:

    1. **Long-throttle success gate**: at least ``check_interval_s`` since the
       last *successful* fetch (or never-fetched). Avoids hammering the
       GitHub unauthenticated API (60 req/h per-IP limit).
    2. **Short-backoff error gate**: at least :data:`_RETRY_INTERVAL_S`
       seconds since the last *failed* fetch. Lets a one-off network blip
       retry within minutes instead of locking the user out for a full day —
       the previous behaviour (Bug F).

    Clock-skew (negative elapsed) lets the check proceed on either gate.
    """
    if not cfg.update.auto_check:
        return False
    now = current_unix_ts() if now_ts is None else now_ts
    # Gate 1 — long throttle on success.
    elapsed_ok = now - cfg.update.last_check_ts
    if elapsed_ok < 0 or elapsed_ok >= cfg.update.check_interval_s:
        return True
    # Gate 2 — short backoff on the most recent error.
    if cfg.update.last_error_ts:
        gap = now - cfg.update.last_error_ts
        if gap < 0 or gap >= _RETRY_INTERVAL_S:
            return True
    return False


def retry_delay_for(cfg: AppConfig, *, now_ts: int | None = None) -> int:
    """Seconds until the next allowed retry after the last failed check.

    Returns ``0`` if a retry is allowed right now (or no error is recorded).
    Pure / side-effect-free; callers use it only for surfacing ETA in the UI.
    """
    if not cfg.update.last_error_ts:
        return 0
    now = current_unix_ts() if now_ts is None else now_ts
    gap = now - cfg.update.last_error_ts
    if gap < 0 or gap >= _RETRY_INTERVAL_S:
        return 0
    return _RETRY_INTERVAL_S - gap


def check_for_update(
    cfg: AppConfig,
    *,
    current_version: str,
    skip_version: str | None = None,
    owner: str = "pumni",
    repo: str = "Sky-Player",
    timeout: float = 5.0,
    channel: str | None = None,
) -> UpdateCheckResult:
    """Wrap :func:`sky_music.domain.update_checker.fetch_latest_release` with
    config-driven defaults. Does NOT persist the check timestamp — callers
    must call :func:`record_successful_check` after a non-erroring fetch.

    ``channel``: when ``None`` (default), uses ``cfg.update.channel``.
    """
    skip = skip_version if skip_version is not None else cfg.update.skip_version
    ch = channel if channel is not None else cfg.update.channel
    return fetch_latest_release(
        owner=owner,
        repo=repo,
        current_version=current_version,
        skip_version=skip or None,
        timeout=timeout,
        channel=ch,
    )


def record_successful_check(cfg: AppConfig, *, now_ts: int | None = None) -> None:
    """Persist ``last_check_ts`` to config.json after a successful fetch.

    Also clears ``last_error_ts`` so the short-backoff gate resets. Should
    only be called when the fetch itself did not raise / return an error
    result, regardless of whether a newer version was found.
    """
    ts = current_unix_ts() if now_ts is None else now_ts
    persist_update_check_ts(cfg, ts)
    if cfg.update.last_error_ts:
        persist_update_error_ts(cfg, 0)


def record_check_error(cfg: AppConfig, *, now_ts: int | None = None) -> None:
    """Persist ``last_error_ts`` after a failed fetch so the short-backoff
    gate in :func:`should_auto_check` can schedule an early retry.

    Pure setter; does not touch ``last_check_ts`` (a failed fetch is not a
    successful check). Idempotent if called repeatedly with the same ts.
    """
    ts = current_unix_ts() if now_ts is None else now_ts
    persist_update_error_ts(cfg, ts)


def record_skip(cfg: AppConfig, version: str) -> None:
    """Persist the skip-version marker; pass empty string to clear it."""
    persist_update_skip_version(cfg, version)



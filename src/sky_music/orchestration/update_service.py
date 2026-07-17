"""Update notifier orchestrator.

Glue layer that ties together the pure version-check domain logic
(:mod:`sky_music.domain.update_checker`), the persistence layer
(:mod:`sky_music.config` for skip-version / last-check timestamps), and the
Windows-side installer (:mod:`sky_music.infrastructure.update_installer`).

The UI only needs to:
  1. Call :func:`should_auto_check` before launching the background check.
  2. Call :func:`check_for_update` in a background thread / worker.
  3. On completion, inspect the returned :class:`UpdateCheckResult`; if there
     is a newer version the user has not skipped, surface it via
     :class:`sky_music.ui.textual_app.modals.UpdateModal`.
  4. When the user picks "download", call :func:`download_and_verify_update`
     to fetch and stage the update. On success, call
     :func:`apply_staged_update` to write + launch the detached .cmd and exit.

This module never blocks the dispatcher or UI thread; callers run network
operations in their own worker.
"""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

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
from sky_music.infrastructure.update_installer import (
    StagedUpdate,
    UpdateInstallerError,
    apply_update_and_restart,
    fetch_sha256_sidecar,
    install_dir_for_frozen,
    post_update_flag_path,
    stage_update,
)


@dataclass(frozen=True, slots=True)
class DownloadOutcome:
    """Result of attempting to download+verify an update.

    ``staged`` is set on success. ``error`` is set on failure (either stage).
    Staging may fail for: missing asset URL, download I/O error, SHA256
    mismatch, zip extraction / zip-slip rejection, or non-Windows apply call.
    """

    staged: StagedUpdate | None
    error: str | None = None


def current_unix_ts() -> int:
    """Return ``int(time.time())`` — isolated for testability."""
    return int(time.time())


# Backoff ladder for retries after a failed fetch (network blip / rate limit).
# Short and capped so a user on a flaky link still gets notified within an
# hour, while never hammering the GitHub unauthenticated API endpoint
# (rate-limited to 60 req/h per-IP). Replaces the previous behaviour where a
# single failed check silently locked the user out of update notifications
# until the long ``check_interval_s`` (24h default) elapses.
_RETRY_INTERVALS_S: tuple[int, ...] = (300, 900, 3600)  # 5m, 15m, 1h
_RETRY_FLOOR_S: int = 3600  # after the ladder, wait 1h between attempts


def should_auto_check(cfg: AppConfig, *, now_ts: int | None = None) -> bool:
    """True iff automatic update check should fire right now.

    Two gates, OR'd together, behind a global opt-in:

    0. **Opt-in gate**: ``auto_check`` AND ``update_choice_made``. The app
       never contacts GitHub on the user's behalf until they have either
       accepted the first-run prompt or toggled the Settings checkbox — this
       is the modern best-practice default (no silent "phoning home" on first
       launch, GDPR/privacy aligned).

    1. **Long-throttle success gate**: at least ``check_interval_s`` since the
       last *successful* fetch (or never-fetched). Avoids hammering the
       GitHub unauthenticated API (60 req/h per-IP limit).
    2. **Short-backoff error gate**: at least one rung on the
       :data:`_RETRY_INTERVALS_S` ladder since the last *failed* fetch. Lets a
       one-off network blip retry within minutes instead of locking the user
       out for a full day.

    Both gates also respect the global ``auto_check`` toggle. Clock-skew
    (negative elapsed) lets the check proceed on either gate.
    """
    if not cfg.update.auto_check or not cfg.update.update_choice_made:
        return False
    now = current_unix_ts() if now_ts is None else int(now_ts)
    # Gate 1 — long throttle on success.
    elapsed_ok = now - cfg.update.last_check_ts
    if elapsed_ok < 0 or elapsed_ok >= cfg.update.check_interval_s:
        return True
    # Gate 2 — short backoff on the most recent error.
    if cfg.update.last_error_ts:
        gap = now - cfg.update.last_error_ts
        if gap < 0:
            return True
        cap = _RETRY_INTERVALS_S[-1] if gap >= sum(_RETRY_INTERVALS_S) else _RETRY_FLOOR_S
        idx = 0
        cumulative = 0
        while idx < len(_RETRY_INTERVALS_S):
            cumulative += _RETRY_INTERVALS_S[idx]
            if gap < cumulative:
                return False
            idx += 1
        return gap >= cap
    return False


def retry_delay_for(cfg: AppConfig, *, now_ts: int | None = None) -> int:
    """Seconds until the next allowed retry after the last failed check.

    Returns ``0`` if a retry is allowed right now (or no error is recorded).
    Pure / side-effect-free; callers use it only for surfacing ETA in the UI.
    """
    if not cfg.update.last_error_ts:
        return 0
    now = current_unix_ts() if now_ts is None else int(now_ts)
    gap = now - cfg.update.last_error_ts
    if gap < 0:
        return 0
    cumulative = 0
    for rung in _RETRY_INTERVALS_S:
        cumulative += rung
        if gap < cumulative:
            return cumulative - gap
    # Past the ladder — wait until the floor elapses since the last rung.
    last_rung_total = sum(_RETRY_INTERVALS_S)
    if gap < last_rung_total + _RETRY_FLOOR_S:
        return last_rung_total + _RETRY_FLOOR_S - gap
    return 0


def check_for_update(
    cfg: AppConfig,
    *,
    current_version: str,
    skip_version: str | None = None,
    owner: str = "pumni",
    repo: str = "Sky-Player",
    timeout: float = 5.0,
    include_prerelease: bool | None = None,
) -> UpdateCheckResult:
    """Wrap :func:`sky_music.domain.update_checker.fetch_latest_release` with
    config-driven defaults. Does NOT persist the check timestamp — callers
    must call :func:`record_successful_check` after a non-erroring fetch.

    ``include_prerelease``: when ``None`` (default), follows the policy
    "auto path = stable only, manual path = stable only". Callers that want to
    surface pre-releases (e.g. a future "include pre-releases" toggle in
    Update Settings) can pass ``True``; explicit ``False`` is also honoured.
    """
    skip = skip_version if skip_version is not None else cfg.update.skip_version
    return fetch_latest_release(
        owner=owner,
        repo=repo,
        current_version=current_version,
        skip_version=skip or None,
        timeout=timeout,
        include_prerelease=bool(include_prerelease),
    )


def record_successful_check(cfg: AppConfig, *, now_ts: int | None = None) -> None:
    """Persist ``last_check_ts`` to config.json after a successful fetch.

    Also clears ``last_error_ts`` so the short-backoff gate resets. Should
    only be called when the fetch itself did not raise / return an error
    result, regardless of whether a newer version was found.
    """
    ts = current_unix_ts() if now_ts is None else int(now_ts)
    persist_update_check_ts(cfg, ts)
    if cfg.update.last_error_ts:
        persist_update_error_ts(cfg, 0)


def record_check_error(cfg: AppConfig, *, now_ts: int | None = None) -> None:
    """Persist ``last_error_ts`` after a failed fetch so the short-backoff
    gate in :func:`should_auto_check` can schedule an early retry.

    Pure setter; does not touch ``last_check_ts`` (a failed fetch is not a
    successful check). Idempotent if called repeatedly with the same ts.
    """
    ts = current_unix_ts() if now_ts is None else int(now_ts)
    persist_update_error_ts(cfg, ts)


def record_skip(cfg: AppConfig, version: str) -> None:
    """Persist the skip-version marker; pass empty string to clear it."""
    persist_update_skip_version(cfg, version)


def download_and_verify_update(
    release: UpdateInfo,
    *,
    install_dir: Path | None = None,
    staging_parent: Path | None = None,
    timeout: float = 60.0,
    progress: Callable[[int, int | None], None] | None = None,
) -> DownloadOutcome:
    """Fetch, (optionally) verify, and stage an update zip.

    When ``install_dir`` is provided, the staging directory is created as a
    versioned sibling (``Sky-Player-v{version}``) on the same volume, enabling
    an atomic rename swap during apply.

    Pulls the sidecar ``.sha256`` URL from ``release.sha256_url``. If the
    sidecar is found and fetch succeeds, ``stage_update`` verifies the
    downloaded zip against it before extracting. If the sidecar is missing,
    the download is staged without a checksum.

    ``staging_parent`` defaults to a ``sky-updates`` subdir of the system tmp.
    """
    download_url = getattr(release, "download_url", "")
    if not download_url:
        return DownloadOutcome(staged=None, error="release has no download asset")

    versioned_dir = install_dir is not None
    if staging_parent is None:
        if install_dir is not None:
            staging_parent = install_dir.resolve().parent
        else:
            import tempfile
            staging_parent = Path(tempfile.gettempdir()) / "sky-updates"
    staging_parent.mkdir(parents=True, exist_ok=True)

    sha256_url = getattr(release, "sha256_url", "") or ""
    sha256_sum: str | None = None
    if sha256_url:
        sha256_sum = fetch_sha256_sidecar(sha256_url, timeout=timeout)
        if sha256_sum is None:
            return DownloadOutcome(
                staged=None,
                error="SHA256 checksum unavailable — refusing insecure download",
            )
    # sha256_url empty = release ships no sidecar; proceed without verification
    try:
        staged = stage_update(
            release,
            staging_parent=staging_parent,
            timeout=timeout,
            sha256_sum=sha256_sum,
            versioned_dir=versioned_dir,
            progress=progress,
        )
    except UpdateInstallerError as exc:
        return DownloadOutcome(staged=None, error=str(exc))
    return DownloadOutcome(staged=staged, error=None if staged else "unknown")


def apply_staged_update(
    staged: StagedUpdate,
    *,
    install_dir: Path | None = None,
    post_update_flag: Path | None = None,
) -> NoReturn:
    """Write the apply batch, launch it detached, and exit.

    Defaults ``install_dir`` and ``post_update_flag`` to the frozen-build
    locations under ``sys.executable``. Outside a frozen build (running from
    source), the caller must supply an explicit ``install_dir``.
    """
    if install_dir is None:
        install_dir = install_dir_for_frozen()
    if post_update_flag is None:
        post_update_flag = post_update_flag_path(install_dir)
    apply_update_and_restart(
        staging_dir=staged.staging_dir,
        install_dir=install_dir,
        post_update_flag=post_update_flag,
    )

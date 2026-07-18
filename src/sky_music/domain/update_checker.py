"""Version-check domain logic — pure, network-injectable, unit-testable.

This module knows nothing about Windows, file I/O, or the running app. It only:
- Parses GitHub Releases API responses (a JSON dict).
- Compares PEP 440 versions via ``packaging.version``.
- Returns typed ``UpdateInfo`` / check results.

Network access is concentrated in ``fetch_latest_release`` which takes an
injectable ``opener`` so tests can substitute a stub instead of hitting the
network. Keep this module free of side effects.
"""

from __future__ import annotations

import json
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Protocol
from urllib.request import Request, urlopen

from packaging.version import InvalidVersion, Version

AssetPredicate = Callable[[dict[str, Any]], bool]

DEFAULT_OWNER: str = "pumni"
DEFAULT_REPO: str = "Sky-Player"
GITHUB_API: str = "https://api.github.com/repos"
FETCH_TIMEOUT_S: float = 5.0
USER_AGENT: str = "sky-player-update-checker"


@dataclass(frozen=True, slots=True)
class UpdateInfo:
    """Metadata about a newer release available upstream.

    ``download_url`` points at a release asset the installer can pull. For
    GitHub Releases this is the ``browser_download_url`` of a chosen asset.
    Empty string if no suitable asset was found (caller should fall back to
    opening the release HTML page).

    ``sha256_url`` points at a sidecar ``.sha256`` asset if one exists on the
    release. Used by the installer to verify the downloaded zip. Empty string
    when the release ships no checksum sidecar (the caller may then choose to
    skip verification or refuse the update).
    """

    latest_version: str
    download_url: str
    release_notes: str
    html_url: str
    published_at: str
    sha256_url: str = ""


@dataclass(frozen=True, slots=True)
class UpdateCheckResult:
    """Outcome of an update check.

    ``update`` is ``None`` when the running version is current, when the user
    has skipped this version, or when fetching failed (``error`` set).
    """

    update: UpdateInfo | None
    current_version: str
    error: str | None = None


class _Opener(Protocol):
    """Subset of ``urllib.request.OpenerDirector`` used here.

    Defining a minimal protocol keeps the surface narrow and lets tests pass
    a callable that matches the call shape ``opener(url, timeout=...)``.
    """

    def __call__(self, url: str | Request, *, timeout: float = ...) -> Any: ...


def parse_version(value: str) -> Version | None:
    """Return ``Version`` or ``None`` for unparseable strings.

    ``packaging.version.Version`` accepts PEP 440 versions. Pre-releases
    (``2.4.0rc1``, ``2.4.0-rc1``) are represented and compare strictly less
    than the same ``major.minor.patch``.
    """
    try:
        return Version(value)
    except InvalidVersion:
        return None


def is_newer(latest: str, current: str) -> bool:
    """True iff ``latest`` is a strictly greater version than ``current``.

    Returns ``False`` when either version string is unparseable — never
    raises, since update logic should degrade to ``no update``.
    """
    latest_v = parse_version(latest)
    current_v = parse_version(current)
    if latest_v is None or current_v is None:
        return False
    return latest_v > current_v


def is_prerelease(version: str) -> bool:
    """True iff ``version`` parses as a PEP 440 pre-release (rc/dev/a/b/post-dev).

    ``packaging.version.Version.is_prerelease`` returns True for any version
    that carries an ``a``/``b``/``rc``/``.dev`` segment. Used by
    :func:`parse_release_payload` to suppress pre-release tags unless the
    caller opts in via ``include_prerelease`` — auto-check/auto-apply default
    to stable-channel behaviour, which is the modern best-practice default
    (Squirrel/Sparkle/VS Code never surface pre-releases to stable users).
    """
    v = parse_version(version)
    if v is None:
        return False
    return v.is_prerelease


def _strip_leading_v(tag: str) -> str:
    """GitHub release tags typically look like ``v2.3.0``; strip the ``v``.

    Accepts odd capitalizations; leaves malformed inputs unchanged.
    """
    if not tag:
        return ""
    lowered = tag.strip()
    if lowered[:1].lower() == "v" and len(lowered) > 1:
        return lowered[1:]
    return lowered


def _select_download_asset(
    assets: list[dict[str, Any]],
    *,
    asset_predicate: AssetPredicate | None = None,
) -> tuple[str, str]:
    """Pick (download_url, sha256_url) for the release.

    Returns ``("", "")`` when assets are missing. The default heuristic for the
    download:
      1. First asset matching ``asset_predicate`` (if provided).
      2. Else the first ``.zip`` asset.
      3. Else the first asset of any kind.

    For ``sha256_url``, returns the first ``.sha256`` browser_download_url on
    the release (or empty when none ships a sidecar).
    """
    if not assets:
        return "", ""
    if asset_predicate is not None:
        for asset in assets:
            if asset_predicate(asset):
                url = asset.get("browser_download_url")
                if isinstance(url, str) and url:
                    download = url
                    sha256 = _select_sha256_url(assets)
                    return download, sha256
    for asset in assets:
        url = asset.get("browser_download_url")
        name = asset.get("name", "")
        if not isinstance(url, str) or not url:
            continue
        if isinstance(name, str) and name.lower().endswith(".zip"):
            return url, _select_sha256_url(assets)
    for asset in assets:
        url = asset.get("browser_download_url")
        if isinstance(url, str) and url:
            return url, _select_sha256_url(assets)
    return "", _select_sha256_url(assets)


def _select_sha256_url(assets: list[dict[str, Any]]) -> str:
    """Return the first ``.sha256`` browser_download_url, or empty string."""
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        name = asset.get("name")
        url = asset.get("browser_download_url")
        if (
            isinstance(name, str)
            and isinstance(url, str)
            and name.lower().endswith(".sha256")
            and url
        ):
            return url
    return ""


def parse_release_payload(
    payload: dict[str, Any],
    *,
    current_version: str,
    skip_version: str | None = None,
    asset_predicate: AssetPredicate | None = None,
    include_prerelease: bool = False,
) -> UpdateCheckResult:
    """Convert a GitHub Releases API dict into an ``UpdateCheckResult``.

    Tolerates missing fields by returning an error result; never raises.

    ``skip_version`` (if set and equal to the latest version) suppresses the
    update — this is how ``Skip this version`` from config is honored.

    ``include_prerelease`` (default ``False``) gates pre-release tags. When
    ``False``, a tag like ``v2.5.0rc1`` is reported as "no update" so
    stable-channel users never get pushed a release candidate via
    auto-check/auto-apply. Manual check (UI ``u`` command) may opt in by
    passing ``True`` — the caller decides whether to surface pre-releases.
    """
    current = current_version or ""
    tag = payload.get("tag_name")
    if not isinstance(tag, str) or not tag:
        return UpdateCheckResult(update=None, current_version=current, error="missing tag_name")
    latest = _strip_leading_v(tag)
    if not is_newer(latest, current):
        return UpdateCheckResult(update=None, current_version=current)
    if not include_prerelease and is_prerelease(latest):
        # Stable-channel mode: do not surface pre-releases. We treat this as
        # "no update" rather than an error so the caller's throttle logic
        # still records a successful (no-op) check against the stable tag.
        return UpdateCheckResult(update=None, current_version=current)
    if skip_version is not None and skip_version == latest:
        return UpdateCheckResult(update=None, current_version=current)

    html_url = payload.get("html_url")
    if not isinstance(html_url, str):
        html_url = ""
    published_at = payload.get("published_at")
    if not isinstance(published_at, str):
        published_at = ""
    body = payload.get("body")
    release_notes = body if isinstance(body, str) else ""

    assets_raw = payload.get("assets")
    assets: list[dict[str, Any]] = (
        [a for a in assets_raw if isinstance(a, dict)] if isinstance(assets_raw, list) else []
    )
    download_url, sha256_url = _select_download_asset(assets, asset_predicate=asset_predicate)

    return UpdateCheckResult(
        update=UpdateInfo(
            latest_version=latest,
            download_url=download_url,
            release_notes=release_notes,
            html_url=html_url,
            published_at=published_at,
            sha256_url=sha256_url,
        ),
        current_version=current,
    )


def fetch_latest_release(
    *,
    owner: str = DEFAULT_OWNER,
    repo: str = DEFAULT_REPO,
    current_version: str,
    skip_version: str | None = None,
    timeout: float = FETCH_TIMEOUT_S,
    opener: _Opener | None = None,
    asset_predicate: AssetPredicate | None = None,
    include_prerelease: bool = False,
) -> UpdateCheckResult:
    """Fetch ``releases/latest`` from GitHub and return an update check result.

    Network failures (DNS, HTTP 4xx/5xx, rate-limit, timeouts, malformed JSON)
    are returned as an error result with ``update=None`` — they never raise.
    The caller decides whether to surface errors to the user; auto-check ignores
    them silently.

    ``include_prerelease`` (default ``False``) is forwarded to
    :func:`parse_release_payload` so the stable channel never surfaces rc/dev
    tags unless the caller explicitly opts in (e.g. a future "show
    pre-releases" opt-in in Update Settings).
    """
    if include_prerelease:
        url = f"{GITHUB_API}/{owner}/{repo}/releases?per_page=10"
    else:
        url = f"{GITHUB_API}/{owner}/{repo}/releases/latest"
    req = Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/vnd.github.v3+json"})
    open_with = opener or urlopen
    try:
        with open_with(req, timeout=timeout) as response:  # type: ignore[arg-type]
            raw = response.read()
    except Exception as exc:
        return UpdateCheckResult(update=None, current_version=current_version, error=str(exc))
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        return UpdateCheckResult(update=None, current_version=current_version, error=str(exc))
    if include_prerelease and isinstance(payload, list):
        best_release = None
        best_version = None
        for release in payload:
            if not isinstance(release, dict):
                continue
            if release.get("draft"):
                continue
            
            tag = release.get("tag_name")
            if not isinstance(tag, str) or not tag:
                continue
                
            latest = _strip_leading_v(tag)
            v = parse_version(latest)
            if v is None:
                continue
                
            if best_version is None or v > best_version:
                best_version = v
                best_release = release
                
        if best_release is None:
            return UpdateCheckResult(
                update=None,
                current_version=current_version,
                error="no valid releases found",
            )
        payload = best_release

    if not isinstance(payload, dict):
        return UpdateCheckResult(
            update=None,
            current_version=current_version,
            error="unexpected non-object payload",
        )
    return parse_release_payload(
        payload,
        current_version=current_version,
        skip_version=skip_version,
        asset_predicate=asset_predicate,
        include_prerelease=include_prerelease,
    )

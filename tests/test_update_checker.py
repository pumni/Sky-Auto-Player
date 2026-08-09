"""Unit tests for ``sky_music.domain.update_checker``.

Domain parsing is pure. Orchestration network tests stub the opener with a
context-manager that returns an in-memory JSON payload.
"""

from __future__ import annotations

import json
from typing import Any

import pytest

from sky_music.domain.update_checker import (
    is_newer,
    parse_release_payload,
    parse_version,
)
from sky_music.orchestration.update_service import fetch_latest_release


def test_domain_update_checker_has_no_network_imports() -> None:
    import ast
    from pathlib import Path

    import sky_music.domain.update_checker as checker_module

    source = Path(checker_module.__file__).read_text(encoding="utf-8")
    tree = ast.parse(source)
    imported_roots = {
        alias.name.split(".", 1)[0]
        for node in ast.walk(tree)
        if isinstance(node, ast.Import)
        for alias in node.names
    }
    imported_roots.update(
        node.module.split(".", 1)[0]
        for node in ast.walk(tree)
        if isinstance(node, ast.ImportFrom) and node.module
    )

    assert "urllib" not in imported_roots


class _StubResponse:
    """Minimal context-manager mimicking ``urlopen``'s returned file-like."""

    def __init__(self, body: bytes) -> None:
        self._body = body
        self.headers: dict[str, str] = {}

    def __enter__(self) -> _StubResponse:
        return self

    def __exit__(self, *_: object) -> None:
        return None

    def read(self, _size: int = -1) -> bytes:
        return self._body


def _stub_opener(payload: dict[str, Any]):
    def opener(url: Any, *, timeout: float = 0.0) -> _StubResponse:
        _ = url, timeout
        return _StubResponse(json.dumps(payload).encode("utf-8"))

    return opener


def _stub_opener_bytes(body: bytes):
    def opener(url: Any, *, timeout: float = 0.0) -> _StubResponse:
        _ = url, timeout
        return _StubResponse(body)

    return opener


# ── parse_version / is_newer ─────────────────────────────────────────────────.


@pytest.mark.parametrize(
    ("value", "expected_ok"),
    [
        ("2.3.0", True),
        ("v2.3.0", True),
        ("2.4.0", True),
        ("2.4.0rc1", True),
        ("2.4.0-rc1", True),
        ("garbage", False),
        ("", False),
        ("2.x.0", False),
    ],
)
def test_parse_version(value: str, expected_ok: bool) -> None:
    assert (parse_version(value) is not None) is expected_ok


def test_is_newer_strict() -> None:
    assert is_newer("2.4.0", "2.3.0") is True
    assert is_newer("2.3.0", "2.3.0") is False
    assert is_newer("2.2.4", "2.3.0") is False
    assert is_newer("2.4.0rc1", "2.3.0") is True
    assert is_newer("2.4.0", "2.4.0rc1") is True
    assert is_newer("garbage", "2.3.0") is False
    assert is_newer("2.4.0", "garbage") is False
    assert is_newer("", "") is False


# ── parse_release_payload ──────────────────────────────────────────────────────.


def _make_release(tag: str, *, assets: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    version = tag.removeprefix("v")
    return {
        "tag_name": tag,
        "html_url": f"https://github.com/pumni/Sky-Auto-Player/releases/tag/{tag}",
        "published_at": "2024-01-01T00:00:00Z",
        "body": "Release notes body",
        "assets": assets if assets is not None else [
            {"name": f"Sky-Auto-Player-v{version}.zip", "browser_download_url": "https://api.github.com/zip"},
            {"name": f"Sky-Auto-Player-v{version}.zip.sha256", "browser_download_url": "https://api.github.com/sha"},
            {"name": "MANIFEST.json", "browser_download_url": "https://api.github.com/manifest"},
        ],
    }


def test_parse_release_payload_newer_version_pick_zip() -> None:
    payload = _make_release(
        "v2.4.0",
        assets=[
            {"name": "Sky-Auto-Player-v2.4.0.zip", "browser_download_url": "https://x/y.zip"},
            {"name": "Sky-Auto-Player-v2.4.0.zip.sha256", "browser_download_url": "https://x/y.zip.sha256"},
            {"name": "MANIFEST.json", "browser_download_url": "https://x/MANIFEST.json"},
        ],
    )
    result = parse_release_payload(payload, current_version="2.3.0")
    assert result.error is None
    assert result.update is not None
    assert result.update.latest_version == "2.4.0"
    assert result.update.download_url == "https://x/y.zip"
    assert result.update.sha256_url == "https://x/y.zip.sha256"
    assert result.update.release_notes == "Release notes body"


def test_parse_release_payload_same_version_no_update() -> None:
    payload = _make_release("v2.3.0")
    result = parse_release_payload(payload, current_version="2.3.0")
    assert result.update is None
    assert result.error is None


def test_parse_release_payload_skip_version_suppresses() -> None:
    payload = _make_release("v2.4.0")
    result = parse_release_payload(payload, current_version="2.3.0", skip_version="2.4.0")
    assert result.update is None
    assert result.error is None


# ── pre-release gating ────────────────────────────────────────────────────────


def test_is_prerelease_detects_common_forms() -> None:
    from sky_music.domain.update_checker import is_prerelease

    assert is_prerelease("2.4.0rc1") is True
    assert is_prerelease("2.4.0-rc1") is True
    assert is_prerelease("2.4.0a1") is True
    assert is_prerelease("2.4.0b2") is True
    assert is_prerelease("2.4.0.dev0") is True
    assert is_prerelease("2.4.0") is False  # stable
    assert is_prerelease("2.4.0.post1") is False  # post is not pre
    assert is_prerelease("garbage") is False  # unparseable → False (safe default)


def test_parse_release_payload_prerelease_default_suppressed() -> None:
    """Auto-/stable-channel default: rc tag newer than current → no update.

    This is the key guard against accidentally auto-applying release
    candidates to stable users via auto_check / auto_apply.
    """
    payload = _make_release("v2.5.0rc1")
    result = parse_release_payload(payload, current_version="2.4.0")
    assert result.error is None
    assert result.update is None


def test_parse_release_payload_prerelease_opt_in_surfaces() -> None:
    """When the caller opts in via include_prerelease=True, rc tags surface."""
    payload = _make_release("v2.5.0rc1")
    result = parse_release_payload(payload, current_version="2.4.0", include_prerelease=True)
    assert result.update is not None
    assert result.update.latest_version == "2.5.0rc1"


def test_parse_release_payload_stable_tag_unaffected_by_gating() -> None:
    """Stable tags must not regress when include_prerelease is left False."""
    payload = _make_release("v2.5.0")
    result = parse_release_payload(payload, current_version="2.4.0")
    assert result.update is not None
    assert result.update.latest_version == "2.5.0"

def test_fetch_latest_release_forward_channel(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """``fetch_latest_release`` must forward ``channel`` to use the correct policy."""
    # Test with beta channel (should include pre-releases)
    payload = _make_release("v2.5.0rc1")
    result = fetch_latest_release(
        current_version="2.4.0",
        opener=_stub_opener(payload),
        channel="beta",
    )
    assert result.update is not None
    assert result.update.latest_version == "2.5.0rc1"

    # Same payload, default stable channel → suppressed.
    result2 = fetch_latest_release(
        current_version="2.4.0",
        opener=_stub_opener(payload),
    )
    assert result2.update is None
    assert result2.error is None  # suppressed, not error: stable-channel no-op


def test_fetch_latest_release_beta_channel_list(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """``fetch_latest_release`` must handle a list of releases when channel=beta,
    picking the highest non-draft release.
    """
    payload = [
        _make_release("v2.4.0"),
        _make_release("v2.5.0-rc1"),
        _make_release("v2.5.0-rc2"),
        {**_make_release("v2.6.0"), "draft": True},
    ]
    
    def fake_opener(url: Any, *, timeout: Any = None) -> _StubResponse:
        assert "releases?per_page=10" in url.full_url
        return _StubResponse(json.dumps(payload).encode("utf-8"))

    result = fetch_latest_release(
        current_version="2.4.0",
        opener=fake_opener,
        channel="beta",
    )
    assert result.update is not None
    assert result.update.latest_version == "2.5.0-rc2"


def test_parse_release_payload_older_tag_is_no_update() -> None:
    payload = _make_release("v2.2.0")
    result = parse_release_payload(payload, current_version="2.3.0")
    assert result.update is None
    assert result.error is None


def test_parse_release_payload_missing_tag_name_is_error() -> None:
    result = parse_release_payload({"html_url": "x"}, current_version="2.3.0")
    assert result.update is None
    assert result.error == "missing tag_name"


def test_parse_release_payload_rejects_noncanonical_assets() -> None:
    payload = _make_release(
        "v2.4.0",
        assets=[{"name": "Sky-Auto-Player-v2.4.0.tar.gz", "browser_download_url": "https://x/y.tar.gz"}],
    )
    result = parse_release_payload(payload, current_version="2.3.0")
    assert result.update is None
    assert result.error is not None


def test_parse_release_payload_rejects_missing_assets() -> None:
    payload = _make_release("v2.4.0", assets=[])
    result = parse_release_payload(payload, current_version="2.3.0")
    assert result.update is None
    assert result.error is not None


def test_parse_release_payload_asset_predicate_cannot_select_noncanonical_zip() -> None:
    payload = _make_release(
        "v2.4.0",
        assets=[
            {"name": "Sky-Auto-Player-v2.4.0.zip", "browser_download_url": "https://x/zip"},
            {"name": "Sky-Auto-Player-v2.4.0.zip.sha256", "browser_download_url": "https://x/sha"},
            {"name": "MANIFEST.json", "browser_download_url": "https://x/manifest"},
        ],
    )

    def is_x64(asset: dict[str, Any]) -> bool:
        return "x64" in str(asset.get("name", ""))

    result = parse_release_payload(payload, current_version="2.3.0", asset_predicate=is_x64)
    assert result.update is None
    assert result.error is not None


def test_parse_release_payload_strips_leading_v() -> None:
    payload = _make_release("v2.4.0")
    result = parse_release_payload(payload, current_version="2.3.0")
    assert result.update is not None
    assert result.update.latest_version == "2.4.0"  # no "v" prefix


def test_parse_release_payload_tag_wihtout_v_prefix() -> None:
    # Some repos tag without the leading v; tolerate this.
    payload = _make_release("2.4.0")
    result = parse_release_payload(payload, current_version="2.3.0")
    assert result.update is not None
    assert result.update.latest_version == "2.4.0"


# ── fetch_latest_release ───────────────────────────────────────────────────────.


def test_fetch_latest_release_uses_injected_opener() -> None:
    payload = _make_release(
        "v2.4.0",
        assets=[
            {"name": "Sky-Auto-Player-v2.4.0.zip", "browser_download_url": "https://x/z.zip"},
            {"name": "Sky-Auto-Player-v2.4.0.zip.sha256", "browser_download_url": "https://x/z.zip.sha256"},
            {"name": "MANIFEST.json", "browser_download_url": "https://x/MANIFEST.json"},
        ],
    )
    result = fetch_latest_release(
        current_version="2.3.0",
        opener=_stub_opener(payload),
    )
    assert result.update is not None
    assert result.update.latest_version == "2.4.0"
    assert result.update.download_url == "https://x/z.zip"


def test_fetch_latest_release_request_error_returns_error_result() -> None:
    def raiser(url: Any, *, timeout: float = 0.0) -> Any:
        raise OSError("boom")

    result = fetch_latest_release(current_version="2.3.0", opener=raiser)
    assert result.update is None
    assert result.error is not None
    assert "boom" in result.error


def test_fetch_latest_release_malformed_json_returns_error_result() -> None:
    result = fetch_latest_release(
        current_version="2.3.0",
        opener=_stub_opener_bytes(b"not json%"),
    )
    assert result.update is None
    assert result.error is not None


def test_fetch_latest_release_non_object_payload_returns_error_result() -> None:
    result = fetch_latest_release(
        current_version="2.3.0",
        opener=_stub_opener_bytes(b"[1,2,3]"),
    )
    assert result.update is None
    assert result.error is not None
    assert "non-object" in result.error


def test_fetch_latest_release_skipped_version_returns_no_update() -> None:
    payload = _make_release("v2.4.0")
    result = fetch_latest_release(
        current_version="2.3.0",
        skip_version="2.4.0",
        opener=_stub_opener(payload),
    )
    assert result.update is None
    assert result.error is None




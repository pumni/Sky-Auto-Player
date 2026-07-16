"""Update-flow simulator -- dev-only harness.

Exercises the full Sky Player update detection pipeline without real network
calls or writes to any production filesystem path.

Usage::

    uv run play test-update [--fake-version VERSION] [--scenario SCENARIO]

    # or run directly:
    uv run python src/simulate_update.py --scenario all

Scenarios
---------
available (default)
    GitHub reports a newer release; prints output identical to --check-update.
already-up-to-date
    GitHub confirms the running version is the latest.
skipped
    A newer version exists but is suppressed by skip_version in config.
error
    Simulates a network failure (timeout / DNS error).
download-ok
    Simulates a successful ZIP download + SHA-256 verification, staged to tmp.
download-bad-sha
    Download succeeds but the SHA-256 checksum does not match.
throttled
    should_auto_check returns False because check_interval_s has not elapsed.
all
    Runs every scenario in sequence and prints a PASS / FAIL summary table.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import sys
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# Force UTF-8 output on Windows consoles (mirrors main.py behaviour).
if sys.platform == "win32":
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    except Exception:
        pass

# ---------------------------------------------------------------------------
# Simulation constants
# ---------------------------------------------------------------------------

_CURRENT_VERSION = "2.3.3"    # running version (mirrors _version.py)
_FAKE_NEW_VERSION = "2.4.0"   # version reported as available on GitHub

_ANSI_RESET  = "\033[0m"
_ANSI_GREEN  = "\033[92m"
_ANSI_RED    = "\033[91m"
_ANSI_CYAN   = "\033[96m"
_ANSI_BOLD   = "\033[1m"
_ANSI_DIM    = "\033[2m"


def _g(s: str) -> str: return f"{_ANSI_GREEN}{s}{_ANSI_RESET}"
def _r(s: str) -> str: return f"{_ANSI_RED}{s}{_ANSI_RESET}"
def _c(s: str) -> str: return f"{_ANSI_CYAN}{s}{_ANSI_RESET}"
def _b(s: str) -> str: return f"{_ANSI_BOLD}{s}{_ANSI_RESET}"
def _d(s: str) -> str: return f"{_ANSI_DIM}{s}{_ANSI_RESET}"


# ---------------------------------------------------------------------------
# Lazy domain import
# ---------------------------------------------------------------------------

def _import_domain() -> Any:
    """Import sky_music domain modules; exit with an error if unavailable."""
    try:
        from sky_music.config import AppConfig, UpdateSettings
        from sky_music.domain.update_checker import (
            UpdateCheckResult,
            UpdateInfo,
            parse_release_payload,
        )
        from sky_music.orchestration.update_service import (
            check_for_update,
            download_and_verify_update,
            record_successful_check,
            should_auto_check,
        )
        return {
            "UpdateCheckResult": UpdateCheckResult,
            "UpdateInfo": UpdateInfo,
            "parse_release_payload": parse_release_payload,
            "should_auto_check": should_auto_check,
            "check_for_update": check_for_update,
            "record_successful_check": record_successful_check,
            "download_and_verify_update": download_and_verify_update,
            "AppConfig": AppConfig,
            "UpdateSettings": UpdateSettings,
        }
    except ImportError as exc:
        print(_r(f"[simulate_update] Cannot import sky_music: {exc}"))
        print(_d("  Run:  uv sync  then try again."))
        sys.exit(1)


# ---------------------------------------------------------------------------
# Fake data factories
# ---------------------------------------------------------------------------

def _make_fake_payload(new_version: str) -> dict[str, Any]:
    """Build a GitHub Releases API response dict for *new_version*."""
    tag = f"v{new_version}"
    return {
        "tag_name": tag,
        "html_url": f"https://github.com/pumni/Sky-Player/releases/tag/{tag}",
        "published_at": "2026-07-16T07:00:00Z",
        "body": (
            f"## What's new in {tag}\n\n"
            "- [sim] Dispatch latency reduced by ~12%\n"
            "- [sim] Fixed UI freeze when selecting tracks longer than 10 min\n"
            "- [sim] Added Sky Season 17 support\n"
        ),
        "assets": [
            {
                "name": f"Sky-Player-{tag}.zip",
                "browser_download_url": f"https://fake-cdn.example.com/Sky-Player-{tag}.zip",
            },
            {
                "name": f"Sky-Player-{tag}.zip.sha256",
                "browser_download_url": f"https://fake-cdn.example.com/Sky-Player-{tag}.zip.sha256",
            },
        ],
    }


def _make_fake_zip() -> bytes:
    """Return a minimal in-memory ZIP that mirrors a real release bundle."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("Sky-Player.exe", b"\x00" * 512)
        zf.writestr("CHANGELOG.md",   b"# Simulated update changelog\n")
        zf.writestr("config.json",    b"{}")
    return buf.getvalue()


# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------

def _section(title: str) -> None:
    width = 60
    print()
    print(_b(_c("-" * width)))
    print(_b(_c(f"  {title}")))
    print(_b(_c("-" * width)))


def _result_line(label: str, value: str, ok: bool = True) -> None:
    icon = _g("[OK]") if ok else _r("[--]")
    print(f"  {icon}  {label:30s} {value}")


# ---------------------------------------------------------------------------
# Scenario result type
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class _ScenarioResult:
    name: str
    passed: bool
    detail: str


# ---------------------------------------------------------------------------
# Scenarios
# ---------------------------------------------------------------------------

def _run_available(m: dict[str, Any], new_version: str) -> _ScenarioResult:
    """Scenario: a newer release is available on GitHub."""
    _section("Scenario: available -- newer release found")

    payload = _make_fake_payload(new_version)
    result  = m["parse_release_payload"](payload, current_version=_CURRENT_VERSION)

    if result.update is None:
        detail = "parse_release_payload returned None (expected UpdateInfo)"
        print(_r(f"  FAIL: {detail}"))
        return _ScenarioResult("available", False, detail)

    upd = result.update
    _result_line("current_version", f"v{result.current_version}")
    _result_line("latest_version",  f"v{upd.latest_version}")
    _result_line("download_url",    upd.download_url or "(empty)")
    _result_line("sha256_url",      upd.sha256_url   or "(empty)")
    _result_line("published_at",    upd.published_at  or "(empty)")
    print()
    if upd.release_notes:
        print(_d("  Release notes:"))
        for line in upd.release_notes.splitlines():
            print(_d(f"    {line}"))

    passed = upd.latest_version == new_version
    return _ScenarioResult("available", passed, f"latest={upd.latest_version}")


def _run_already_up_to_date(m: dict[str, Any]) -> _ScenarioResult:
    """Scenario: the running version is already the latest."""
    _section("Scenario: already-up-to-date")

    payload      = _make_fake_payload(_CURRENT_VERSION)
    result       = m["parse_release_payload"](payload, current_version=_CURRENT_VERSION)
    is_current   = result.update is None and result.error is None

    _result_line("update == None (up-to-date)", str(is_current), ok=is_current)
    return _ScenarioResult("already-up-to-date", is_current, "update is None")


def _run_skipped(m: dict[str, Any], new_version: str) -> _ScenarioResult:
    """Scenario: a newer version is available but suppressed by skip_version."""
    _section(f"Scenario: skipped -- skip_version={new_version}")

    payload = _make_fake_payload(new_version)
    result  = m["parse_release_payload"](
        payload,
        current_version=_CURRENT_VERSION,
        skip_version=new_version,   # exact match -- suppressed
    )

    skipped_ok = result.update is None and result.error is None
    _result_line(
        f"skip_version={new_version}",
        f"update is None -> {skipped_ok}",
        ok=skipped_ok,
    )
    return _ScenarioResult("skipped", skipped_ok, f"skip_version={new_version}")


def _run_error(_m: dict[str, Any]) -> _ScenarioResult:
    """Scenario: network failure -- opener raises an OSError."""
    _section("Scenario: error -- network failure")

    import sky_music.domain.update_checker as checker_mod

    def _bad_opener(url: str | Any, *, timeout: float = 0.0) -> Any:
        _ = url, timeout
        raise OSError("Simulated network timeout: connection timed out")

    result = checker_mod.fetch_latest_release(
        current_version=_CURRENT_VERSION,
        opener=_bad_opener,
    )

    has_error = result.error is not None and result.update is None
    error_msg = result.error or "(no error set)"
    _result_line("error is not None", error_msg, ok=has_error)
    _result_line("update is None",    str(result.update is None), ok=result.update is None)
    return _ScenarioResult("error", has_error, error_msg)


def _run_download_ok(m: dict[str, Any], new_version: str, tmp_dir: Path) -> _ScenarioResult:
    """Scenario: ZIP download succeeds and SHA-256 checksum matches."""
    _section("Scenario: download-ok -- successful download and verify")

    import sky_music.infrastructure.update_installer as installer_mod

    zip_bytes  = _make_fake_zip()
    sha256_hex = hashlib.sha256(zip_bytes).hexdigest()

    class _FakeResp:
        def __init__(self, data: bytes, headers: dict[str, str] | None = None) -> None:
            self._buf    = io.BytesIO(data)
            self.headers = headers or {}
        def __enter__(self) -> _FakeResp: return self
        def __exit__(self, *_: object) -> None: pass
        def read(self, size: int = -1) -> bytes:
            return self._buf.read(-1 if size == -1 else size)

    def _fake_opener(url: str, *, timeout: float = 0.0) -> _FakeResp:
        _ = timeout
        if url.endswith(".sha256"):
            return _FakeResp(sha256_hex.encode())
        return _FakeResp(zip_bytes, {"Content-Length": str(len(zip_bytes))})

    original = getattr(installer_mod, "_urlopen_default", None)
    installer_mod._urlopen_default = _fake_opener  # type: ignore[attr-defined]

    UpdateInfo = m["UpdateInfo"]
    release = UpdateInfo(
        latest_version=new_version,
        download_url=f"https://fake-cdn.example.com/Sky-Player-v{new_version}.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url=f"https://fake-cdn.example.com/Sky-Player-v{new_version}.zip.sha256",
    )
    staging_parent = tmp_dir / "staging_ok"
    outcome = m["download_and_verify_update"](release, staging_parent=staging_parent)

    if original is not None:
        installer_mod._urlopen_default = original  # type: ignore[attr-defined]

    ok = outcome.error is None and outcome.staged is not None
    _result_line("error is None",    str(outcome.error  is None),  ok=outcome.error  is None)
    _result_line("staged not None",  str(outcome.staged is not None), ok=outcome.staged is not None)
    if outcome.staged:
        _result_line(
            "staging_dir exists",
            str(outcome.staged.staging_dir.exists()),
            ok=outcome.staged.staging_dir.exists(),
        )
    return _ScenarioResult("download-ok", ok, outcome.error or "OK")


def _run_download_bad_sha(m: dict[str, Any], new_version: str, tmp_dir: Path) -> _ScenarioResult:
    """Scenario: download succeeds but SHA-256 does not match -- refused."""
    _section("Scenario: download-bad-sha -- SHA-256 mismatch")

    import sky_music.infrastructure.update_installer as installer_mod

    zip_bytes = _make_fake_zip()
    bad_sha   = "0" * 64   # intentionally wrong checksum

    class _FakeResp:
        def __init__(self, data: bytes, headers: dict[str, str] | None = None) -> None:
            self._buf    = io.BytesIO(data)
            self.headers = headers or {}
        def __enter__(self) -> _FakeResp: return self
        def __exit__(self, *_: object) -> None: pass
        def read(self, size: int = -1) -> bytes:
            return self._buf.read(-1 if size == -1 else size)

    def _fake_opener(url: str, *, timeout: float = 0.0) -> _FakeResp:
        _ = timeout
        if url.endswith(".sha256"):
            return _FakeResp(bad_sha.encode())
        return _FakeResp(zip_bytes, {"Content-Length": str(len(zip_bytes))})

    original = getattr(installer_mod, "_urlopen_default", None)
    installer_mod._urlopen_default = _fake_opener  # type: ignore[attr-defined]

    UpdateInfo = m["UpdateInfo"]
    release = UpdateInfo(
        latest_version=new_version,
        download_url=f"https://fake-cdn.example.com/Sky-Player-v{new_version}.zip",
        release_notes="",
        html_url="",
        published_at="",
        sha256_url=f"https://fake-cdn.example.com/Sky-Player-v{new_version}.zip.sha256",
    )
    staging_parent = tmp_dir / "staging_bad_sha"
    outcome = m["download_and_verify_update"](release, staging_parent=staging_parent)

    if original is not None:
        installer_mod._urlopen_default = original  # type: ignore[attr-defined]

    # Expect: staged=None, error message contains "sha256"
    ok = (
        outcome.staged is None
        and outcome.error is not None
        and "sha256" in (outcome.error or "")
    )
    _result_line("staged is None",        str(outcome.staged is None), ok=outcome.staged is None)
    _result_line(
        "error contains 'sha256'",
        str("sha256" in (outcome.error or "")),
        ok="sha256" in (outcome.error or ""),
    )
    if outcome.error:
        print(f"  {_d('error:')} {outcome.error}")
    return _ScenarioResult("download-bad-sha", ok, outcome.error or "unexpected OK")


def _run_throttled(m: dict[str, Any]) -> _ScenarioResult:
    """Scenario: should_auto_check returns False because interval has not elapsed."""
    _section("Scenario: throttled -- check_interval_s not yet elapsed")

    AppConfig         = m["AppConfig"]
    UpdateSettings    = m["UpdateSettings"]
    should_auto_check = m["should_auto_check"]

    last_ts  = 1_718_200_000
    interval = 86_400           # 24 h
    now_ts   = last_ts + 3_600  # only 1 h later -- still within throttle window

    cfg    = AppConfig(update=UpdateSettings(
        auto_check=True,
        check_interval_s=interval,
        last_check_ts=last_ts,
    ))
    result  = should_auto_check(cfg, now_ts=now_ts)
    elapsed = now_ts - last_ts

    ok = result is False
    _result_line(
        f"elapsed={elapsed}s < interval={interval}s",
        f"should_auto_check={result}",
        ok=ok,
    )
    return _ScenarioResult("throttled", ok, f"should_auto_check={result}")


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

_ALL_SCENARIOS = [
    "available",
    "already-up-to-date",
    "skipped",
    "error",
    "download-ok",
    "download-bad-sha",
    "throttled",
]


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="play test-update",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--fake-version",
        default=_FAKE_NEW_VERSION,
        metavar="VERSION",
        help=f"version to report as available on GitHub (default: {_FAKE_NEW_VERSION})",
    )
    parser.add_argument(
        "--scenario",
        default="available",
        choices=[*_ALL_SCENARIOS, "all"],
        metavar="SCENARIO",
        help=(
            "simulation scenario to run. Choices: "
            + ", ".join(_ALL_SCENARIOS)
            + ", all  (default: available)"
        ),
    )
    args = parser.parse_args()
    new_version: str = args.fake_version
    scenario: str    = args.scenario

    # Banner
    print()
    print(_b(_c("======================================================")))
    print(_b(_c("    Sky Player -- Update Flow Simulator              ")))
    print(_b(_c("======================================================")))
    print()
    print(f"  {_d('Current version:')} {_b(_CURRENT_VERSION)}")
    print(f"  {_d('Fake new version:')} {_b(new_version)}")
    print(f"  {_d('Scenario:        ')} {_b(scenario)}")

    m = _import_domain()

    import tempfile
    tmp_dir = Path(tempfile.gettempdir()) / "sky-update-sim"
    tmp_dir.mkdir(parents=True, exist_ok=True)

    scenarios_to_run: list[str] = _ALL_SCENARIOS if scenario == "all" else [scenario]
    results: list[_ScenarioResult] = []

    for sc in scenarios_to_run:
        try:
            if sc == "available":
                results.append(_run_available(m, new_version))
            elif sc == "already-up-to-date":
                results.append(_run_already_up_to_date(m))
            elif sc == "skipped":
                results.append(_run_skipped(m, new_version))
            elif sc == "error":
                results.append(_run_error(m))
            elif sc == "download-ok":
                results.append(_run_download_ok(m, new_version, tmp_dir))
            elif sc == "download-bad-sha":
                results.append(_run_download_bad_sha(m, new_version, tmp_dir))
            elif sc == "throttled":
                results.append(_run_throttled(m))
        except Exception as exc:
            results.append(_ScenarioResult(sc, False, f"Exception: {exc}"))
            print(_r(f"\n  [EXCEPTION in {sc}] {exc}"))

    # Summary table (only shown when multiple scenarios ran)
    if len(results) > 1:
        _section("Summary")
        all_ok = True
        for r in results:
            icon = _g("PASS") if r.passed else _r("FAIL")
            print(f"  [{icon}]  {r.name:30s} {_d(r.detail)}")
            if not r.passed:
                all_ok = False
        print()
        if all_ok:
            print(_g("  All scenarios passed [OK]"))
        else:
            print(_r("  One or more scenarios failed [--]"))
        return 0 if all_ok else 1

    # Single-scenario result
    print()
    single = results[0]
    if single.passed:
        print(_g(f"  [OK]  Scenario '{single.name}' passed"))
    else:
        print(_r(f"  [--]  Scenario '{single.name}' failed: {single.detail}"))
    return 0 if single.passed else 1


if __name__ == "__main__":
    sys.exit(main())

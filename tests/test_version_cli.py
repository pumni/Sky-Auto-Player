"""Tests for ``--compare-versions`` CLI command."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest


def _run_compare(current: str, latest: str) -> tuple[int, str, str]:
    """Run the compare-versions command and return (exit_code, stdout, stderr)."""
    # Use the module directly for testing (not frozen exe)
    result = subprocess.run(
        [sys.executable, "-m", "src.main", "--compare-versions", current, latest],
        capture_output=True,
        text=True,
        cwd=Path(__file__).resolve().parents[1],
    )
    return result.returncode, result.stdout, result.stderr


@pytest.mark.parametrize(
    ("current", "latest", "expected_code"),
    [
        # Equal versions
        ("2.4.0", "2.4.0", 0),
        ("2.4.0", "v2.4.0", 0),  # leading v stripped by caller
        ("2.4.0-rc1", "2.4.0-rc1", 0),
        # Latest > Current (newer) -> exit 1
        ("2.3.0", "2.4.0", 1),
        ("2.4.0", "2.4.1", 1),
        ("2.4.0", "2.5.0", 1),
        ("2.4.0", "3.0.0", 1),
        ("2.4.0-rc1", "2.4.0", 1),  # stable > rc
        ("2.4.0a1", "2.4.0b1", 1),  # beta > alpha
        ("2.4.0b1", "2.4.0rc1", 1),  # rc > beta
        ("2.4.0rc1", "2.4.0", 1),   # stable > rc
        ("2.4.0", "2.4.0.post1", 1), # post > stable
        ("2.4.0.dev0", "2.4.0a1", 1), # alpha > dev
        # Latest < Current (older) -> exit 2
        ("2.4.0", "2.3.0", 2),
        ("2.4.1", "2.4.0", 2),
        ("2.4.0", "2.4.0-rc1", 2),  # rc < stable
        ("2.4.0b1", "2.4.0a1", 2),  # alpha < beta
    ],
)
def test_compare_versions(current: str, latest: str, expected_code: int) -> None:
    code, _out, err = _run_compare(current, latest)
    assert code == expected_code, f"Expected {expected_code}, got {code}. stderr: {err}"


def test_compare_versions_invalid_current() -> None:
    code, _out, err = _run_compare("not-a-version", "2.4.0")
    assert code == 3
    assert "invalid version string" in err.lower()


def test_compare_versions_invalid_latest() -> None:
    code, _out, err = _run_compare("2.4.0", "not-a-version")
    assert code == 3
    assert "invalid version string" in err.lower()


def test_compare_versions_both_invalid() -> None:
    code, _out, _err = _run_compare("garbage", "also-garbage")
    assert code == 3


def test_compare_versions_help_shows() -> None:
    """--compare-versions should appear in help text."""
    result = subprocess.run(
        [sys.executable, "-m", "src.main", "--help"],
        capture_output=True,
        text=True,
        cwd=Path(__file__).resolve().parents[1],
    )
    assert "--compare-versions" in result.stdout
    assert "PEP 440" in result.stdout
    assert "exit" in result.stdout.lower()
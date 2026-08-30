from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).parents[1]


def _load_selftest():
    path = ROOT / "src" / "sky_music" / "cli" / "desktop_core_selftest.py"
    spec = importlib.util.spec_from_file_location("phase8_core_selftest_under_test", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load Core selftest")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _fixture_command(scenario: str) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "tests" / "fixtures" / "phase8_core_selftest_child.py"),
        scenario,
    ]


@pytest.mark.parametrize(
    "scenario",
    [
        "exit_before_ready",
        "startup_fatal",
        "malformed",
        "non_utf8",
        "wrong_response_id",
        "request_timeout",
        "shutdown_hang",
    ],
)
def test_core_selftest_protocol_failures_are_bounded(
    tmp_path: Path, scenario: str
) -> None:
    selftest = _load_selftest()
    result = selftest.run_core_selftest(
        command=_fixture_command(scenario), root=tmp_path, timeout_s=0.25
    )
    assert result == 1


def test_core_selftest_missing_executable_fails_closed(tmp_path: Path) -> None:
    selftest = _load_selftest()
    result = selftest.run_core_selftest(
        command=[str(tmp_path / "missing-core.exe")], root=tmp_path, timeout_s=0.25
    )
    assert result == 2


def test_core_selftest_positive_fixture_completes(tmp_path: Path) -> None:
    selftest = _load_selftest()
    result = selftest.run_core_selftest(
        command=_fixture_command("good"), root=tmp_path, timeout_s=0.5
    )
    assert result == 0

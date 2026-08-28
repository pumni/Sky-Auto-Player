"""Windows-only contract tests for the updater acceptance harness."""

from __future__ import annotations

import os
import pathlib
import subprocess

import pytest


_SYNTHETIC_SHARING_VIOLATION = "being used by another process"
_MAX_SELF_TEST_ATTEMPTS = 3


def _run_result_polling_self_test(script: pathlib.Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "pwsh",
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(script),
            "-FromPackage",
            str(script.parent / "unused-from.zip"),
            "-ToPackage",
            str(script.parent / "unused-to.zip"),
            "-SelfTestResultPolling",
        ],
        capture_output=True,
        text=True,
        # The PowerShell self-test has its own five-second polling deadline.
        # This outer guard only covers cold pwsh/Add-Type startup on hosted
        # Windows runners and must not be used to stretch the polling contract.
        timeout=45,
        check=False,
    )


@pytest.mark.windows
def test_result_polling_ignores_intermediate_success() -> None:
    if os.name != "nt":
        pytest.skip("requires Windows PowerShell")

    script = pathlib.Path(__file__).parents[1] / "scripts" / "test_windows_updater_e2e.ps1"
    completed: subprocess.CompletedProcess[str] | None = None
    for _ in range(_MAX_SELF_TEST_ATTEMPTS):
        completed = _run_result_polling_self_test(script)
        if completed.returncode == 0:
            return
        output = f"{completed.stderr}\n{completed.stdout}".casefold()
        if _SYNTHETIC_SHARING_VIOLATION not in output:
            break

    assert completed is not None
    assert completed.returncode == 0, completed.stderr or completed.stdout

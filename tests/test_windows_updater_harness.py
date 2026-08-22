"""Windows-only contract tests for the updater acceptance harness."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest


@pytest.mark.windows
def test_result_polling_ignores_intermediate_success() -> None:
    if os.name != "nt":
        pytest.skip("requires Windows PowerShell")

    script = Path(__file__).parents[1] / "scripts" / "test_windows_updater_e2e.ps1"
    completed = subprocess.run(
        [
            "pwsh",
            "-NoProfile",
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
        timeout=20,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr or completed.stdout

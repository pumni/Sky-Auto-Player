"""Windows acceptance seam for the real Python-to-native updater handoff."""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from sky_music.infrastructure.update_launcher import (
    UpdateLaunchRequest,
    launch_update,
)


@pytest.mark.windows
def test_real_python_launcher_completes_ready_handoff() -> None:
    if os.name != "nt":
        pytest.skip("requires Windows")

    install_root_value = os.environ.get("SKY_ONE_CLICK_INSTALL_ROOT")
    if not install_root_value:
        pytest.skip("harness did not provide SKY_ONE_CLICK_INSTALL_ROOT")
    install_root = Path(install_root_value)
    current_version = os.environ.get("SKY_ONE_CLICK_CURRENT_VERSION", "3.4.4")
    target_version = os.environ.get("SKY_ONE_CLICK_TARGET_VERSION", "3.4.5")

    result = launch_update(
        UpdateLaunchRequest(
            install_root=install_root,
            current_version=current_version,
            target_version=target_version,
            channel="stable",
            restart=True,
        )
    )

    assert result.status == "ready"
    handoff = json.loads((result.run_root / "handoff.json").read_text(encoding="utf-8"))
    assert handoff["state"] == "ready"
    assert handoff["target_version"] == target_version
    assert handoff["updater_pid"] == result.updater_pid

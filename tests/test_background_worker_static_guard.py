from __future__ import annotations

import re
from pathlib import Path


def test_static_drift_guard() -> None:
    src_dir = Path("src/sky_music")
    assert src_dir.exists()

    allowed_thread_pool = {
        "sky_music/ui/picker_background.py",
        "sky_music/ui/textual_app/workers.py",
    }
    
    allowed_process_pool = {
        "sky_music/ui/picker_background.py",
    }
    
    allowed_threading_thread = {
        "sky_music/orchestration/engine.py",
        "sky_music/orchestration/playback_supervisor.py",
        "sky_music/watchdog.py",
        "sky_music/infrastructure/backend.py",
    }

    errors = []

    for file_path in src_dir.glob("**/*.py"):
        rel_path = file_path.relative_to(src_dir.parent).as_posix()
        content = file_path.read_text(encoding="utf-8")

        # 1. ThreadPoolExecutor instantiation guard
        # Match ThreadPoolExecutor(...)
        if "ThreadPoolExecutor(" in content and rel_path not in allowed_thread_pool:
                errors.append(
                    f"ThreadPoolExecutor instantiated in unauthorized file: {file_path.relative_to(src_dir.parent.parent)}"
                )

        # 2. ProcessPoolExecutor instantiation guard
        if "ProcessPoolExecutor(" in content and rel_path not in allowed_process_pool:
                errors.append(
                    f"ProcessPoolExecutor instantiated in unauthorized file: {file_path.relative_to(src_dir.parent.parent)}"
                )

        # 3. threading.Thread instantiation guard
        # We search for threading.Thread or from threading import Thread ... Thread(
        if ("threading.Thread(" in content or " Thread(" in content) and rel_path not in allowed_threading_thread:
                errors.append(
                    f"threading.Thread instantiated in unauthorized file: {file_path.relative_to(src_dir.parent.parent)}"
                )

        # 4. shutdown(wait=False) raw call guard
        # Match shutdown(wait=False) or shutdown(False)
        if re.search(r"shutdown\(\s*(wait\s*=\s*)?False\s*\)", content):
            errors.append(
                f"Raw shutdown(wait=False) call found in: {file_path.relative_to(src_dir.parent.parent)}"
            )

        # 5. run_worker(..., thread=True) guard
        # Match run_worker with thread=True
        if "run_worker(" in content and "thread=True" in content:
            errors.append(
                f"run_worker(..., thread=True) found in: {file_path.relative_to(src_dir.parent.parent)}"
            )

    if errors:
        raise AssertionError("\n".join(errors))

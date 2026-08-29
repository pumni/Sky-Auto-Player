"""Debug-only Tauri helper that execs the repository's exact Python interpreter."""

from __future__ import annotations

import os
import sys
from pathlib import Path


def main() -> int:
    repository_root = Path(__file__).resolve().parents[1]
    core_entrypoint = repository_root / "src" / "core_main.py"
    os.execv(sys.executable, [sys.executable, str(core_entrypoint), *sys.argv[1:]])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())


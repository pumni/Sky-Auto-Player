"""Regenerate ts-rs bindings used by the desktop frontend."""
from __future__ import annotations

import os
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    env = os.environ.copy()
    env["TS_RS_EXPORT_DIR"] = str(ROOT / "desktop" / "src" / "bridge" / "generated")
    env["TS_RS_LARGE_INT"] = "number"
    command = [
        "cargo",
        "test",
        "--manifest-path",
        str(ROOT / "rust" / "Cargo.toml"),
        "-p",
        "sky_desktop_shell",
        "--lib",
        "--all-features",
        "--locked",
    ]
    return subprocess.run(command, cwd=ROOT, env=env, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())

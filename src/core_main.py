"""Frozen/source entrypoint for ``Sky-Auto-Player-Core.exe``."""

from __future__ import annotations

import sys


def _run_tui() -> int:
    """Run the packaged Textual/CLI fallback from this same executable."""
    from main import main as tui_main

    sys.argv = [argument for argument in sys.argv if argument != "--tui"]
    return tui_main()


def _run_desktop_selftest() -> int:
    from sky_music.cli.desktop_core_selftest import run_packaged_core_selftest

    return run_packaged_core_selftest()


def main() -> int:
    if "--tui" in sys.argv[1:]:
        return _run_tui()
    if "--selftest-desktop-core" in sys.argv[1:]:
        return _run_desktop_selftest()
    from sky_music.cli.desktop_core import main as desktop_main

    return desktop_main()

if __name__ == "__main__":
    raise SystemExit(main())

"""Sky Music Player package.

Single source of truth for the runtime version string. Falls back to a
hardcoded literal when ``importlib.metadata`` cannot read the package metadata
(this happens in frozen PyInstaller builds where the ``sky-player`` dist-info
is not always collected).
"""

from __future__ import annotations

import importlib.metadata

__all__ = ["__version__"]

_FALLBACK_VERSION = "2.3.0"


def _resolve_version() -> str:
    try:
        return importlib.metadata.version("sky-player")
    except Exception:
        return _FALLBACK_VERSION


__version__: str = _resolve_version()

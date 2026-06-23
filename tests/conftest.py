"""conftest.py — pytest root configuration.

Ensures that ``src/`` is on ``sys.path`` for every test session so individual
test files do not need to call ``sys.path.insert`` themselves.
"""
from __future__ import annotations

import sys
from pathlib import Path

# Add the src/ directory to sys.path once for the entire session.
# This replaces the per-file `sys.path.insert(0, str(src_dir))` pattern.
_src = Path(__file__).parent.parent / "src"
if str(_src) not in sys.path:
    sys.path.insert(0, str(_src))

"""Pytest configuration for repository-only tooling tests.

The supported application is native Rust/Tauri.  This file deliberately does
not add ``src`` to ``sys.path`` or import a Python product package.
"""

from __future__ import annotations

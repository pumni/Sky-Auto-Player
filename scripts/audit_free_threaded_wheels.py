"""Audit the free-threaded (cp314t) readiness of Sky Player's environment.

Run before ``python -m build_app`` to catch a GIL-disabled regression or a
wheel downgrade in any native runtime dependency. Exits non-zero on any
failure so it can be used as a CI gate.

Why this matters
----------------
Sky Player pins ``.python-version`` to ``3.14+freethreaded`` so the dispatch
spinner and the Textual UI thread run truly in parallel. Native deps must
ship ``cp314t`` wheels — a standard ``cp314`` wheel will fail to import under
a free-threaded interpreter (different ABI), so the runtime import check is
the load-bearing verification for native packages. For pure-python packages
we only enforce the minimum version listed in this file (mirrors pyproject).
"""
from __future__ import annotations

import sys
import sysconfig
from importlib import metadata
from importlib.metadata import PackageNotFoundError

from packaging.specifiers import SpecifierSet
from packaging.version import Version

NAME: str = "Sky Player"
# Mirror pyproject.toml pins. Both `name` (import key) and `version_spec`
# (PEP 440 specifier) are checked at runtime.
RUNTIME_MIN_VERSION: dict[str, str] = {
    "rapidfuzz": ">=3.14.5",
    "textual": ">=8.2.7",
}


def check_interpreter() -> bool:
    build_flag = sysconfig.get_config_var("Py_GIL_DISABLED") == 1
    # CPython 3.13+ exposes `sys._is_gil_enabled()` for runtime introspection.
    # Returns False when the GIL is disabled (build flag = 1 and not toggled
    # back on via `PYTHON_GIL=1` at launch).
    runtime_gil_disabled = not sys._is_gil_enabled()
    print("Interpreter:")
    print(f"  implementation : {sys.implementation.name}")
    print(f"  version        : {sys.version.split()[0]}")
    print(f"  build flag     : {'Py_GIL_DISABLED=1' if build_flag else 'Py_GIL_DISABLED=0'}")
    print(f"  runtime GIL    : {'disabled' if runtime_gil_disabled else 'ENABLED'}")
    return build_flag and runtime_gil_disabled


def check_dep(name: str, min_spec: str) -> bool:
    try:
        dist = metadata.distribution(name)
    except PackageNotFoundError:
        print(f"  [{name}] NOT INSTALLED — run `uv sync` first.")
        return False

    # Enforce minimum version exactly the way pyproject.toml does.
    try:
        version_ok = Version(dist.version) in SpecifierSet(min_spec)
    except Exception as exc:
        print(f"  [{name}] version comparison failed: {exc}")
        return False

    files = list(dist.files or [])
    has_native = any(str(f).endswith((".so", ".pyd", ".dylib")) for f in files)
    kind = "native" if has_native else "pure-python"

    if not version_ok:
        print(f"  [{name}] {dist.version} ({kind}) — fails specifier {min_spec}")
        return False

    print(f"  [{name}] {dist.version} ({kind}, requires {min_spec})")

    if not has_native:
        # Pure-python loads identically under any ABI — version check suffices.
        return True

    # Native dep must import under no-GIL. Successful load proves the wheel
    # was built for the cp314t ABI; a cp314 wheel would not load here.
    try:
        __import__(name)
    except Exception as exc:
        print(f"    [FAIL] import failed: {exc}")
        return False

    return True


def main() -> int:
    print(f"=== {NAME}: free-threaded wheel audit ===\n")
    py_ok = check_interpreter()
    if not py_ok:
        print("\n[FAIL] Interpreter is not free-threaded.")
        print("        Fix: set `.python-version` to `3.14+freethreaded` and `uv sync`.")
        return 1

    print("\nRuntime dependencies:")
    deps_ok = all(check_dep(n, v) for n, v in RUNTIME_MIN_VERSION.items())
    if not deps_ok:
        print("\n[FAIL] One or more runtime dependencies failed the audit.")
        return 1

    print("\n[OK] Environment free-threaded-ready. Safe to invoke `build-app`.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

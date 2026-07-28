"""Patch C1 (ask-first): --selftest-optimize branch in main.py.

The spec comment at Sky-Auto-Player.spec:77-79 claims ``optimize=1``
"preserves assert statements" — factually wrong (Python strips ``__debug__``
and ``assert`` blocks under ``optimize>=1``). This test gates the
``--selftest-optimize`` branch in ``main.py``: it must report
``sys.flags.optimize`` and ``__debug__``, and return non-zero when the
interpreter has ``__debug__ is True`` (dev / pytest / `-O`-absent) so the
frozen-build smoke step catches the contract mismatch before a release ships.

The release-mode pass path is not unit-tested here because Python's
``__debug__`` is a compiler-resolved constant that cannot be turned off at
runtime — simulating it would require either a separate interpreter or a
``subprocess`` round-trip. The ``src/build_app.py`` smoke step exercises
both ``--selftest-textual`` and ``--selftest-optimize`` against the actual
frozen binary on every release build.
"""

from __future__ import annotations

import io
from contextlib import redirect_stderr, redirect_stdout


def test_selftest_optimize_reports_flags_and_fails_under_debug() -> None:
    """``_run_optimize_selftest`` must print ``sys.flags.optimize`` and the
    ``__debug__`` value, and return non-zero when ``__debug__ is True``
    (the pytest default). The frozen build runs the same branch with
    ``__debug__ is False`` and exits zero — the smoke step in
    ``src/build_app.py`` runs both `--selftest-textual` and
    `--selftest-optimize` after packaging so a regression in the spec
    ``optimize`` field cannot ship silently.
    """
    # Import lazily to avoid pulling main.py's CLI parse side effects during
    # collection; main.py also wires ``os.chdir`` for frozen-only paths.
    # ``py-modules = ["main"]`` in pyproject.toml registers it as a top-level
    # module, so ``import main`` is the canonical path.
    import main as main_mod

    stdout, stderr = io.StringIO(), io.StringIO()
    with redirect_stdout(stdout), redirect_stderr(stderr):
        rc = main_mod._run_optimize_selftest()
    out = stdout.getvalue() + stderr.getvalue()

    assert "optimize" in out, (
        f"selftest must report sys.flags.optimize, got: {out!r}"
    )
    assert "__debug__" in out, (
        f"selftest must report the __debug__ flag, got: {out!r}"
    )
    assert "__debug__: True" in out, (
        f"selftest must print the actual __debug__ value (pytest default is True), got: {out!r}"
    )
    # In pytest __debug__ is True, so the selftest must report failure
    # — that's the whole point of catching a contract drift before release.
    assert rc == 1, (
        f"selftest must exit non-zero when __debug__ is True "
        f"(pytest default); got rc={rc}"
    )


def test_selftest_optimize_branch_is_wired_in_main() -> None:
    """The CLI dispatcher in ``main()`` must route ``--selftest-optimize``
    to ``_run_optimize_selftest``. Without this wiring the new smoke step
    in ``src/build_app.py`` cannot be invoked from the frozen binary.
    """
    import main as main_mod

    src = main_mod.__file__ or "<string>"
    if not src.endswith(".py"):
        src += ".py"

    py_text = open(src, encoding="utf-8").read()
    assert "--selftest-optimize" in py_text, (
        "main() must dispatch --selftest-optimize to the selftest branch"
    )
    assert "main_mod._run_optimize_selftest()" in py_text or "_run_optimize_selftest()" in py_text, (
        "main() must call _run_optimize_selftest when --selftest-optimize is passed"
    )

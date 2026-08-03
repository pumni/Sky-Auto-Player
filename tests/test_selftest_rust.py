"""Native extension smoke-test contract."""

from __future__ import annotations

import main as main_mod


def test_rust_selftest_runs_empty_native_schedule_without_input() -> None:
    assert main_mod._run_rust_selftest() == 0


def test_rust_selftest_branch_is_wired() -> None:
    assert "--selftest-rust" in main_mod.main.__code__.co_consts

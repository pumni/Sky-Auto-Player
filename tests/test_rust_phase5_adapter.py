"""Tests for Phase 5: Python Adapter Bridge and feature flag."""

from __future__ import annotations

import os

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]

from sky_music.orchestration.core.rust_adapter import (
    RustInputAdapter,
    is_rust_dispatch_available,
    reset_rust_availability_cache,
)


def test_rust_dispatch_availability() -> None:
    reset_rust_availability_cache()
    available = is_rust_dispatch_available()
    assert available is True


def test_rust_dispatch_feature_flag_disable() -> None:
    reset_rust_availability_cache()
    os.environ["SKY_USE_RUST_DISPATCH"] = "0"
    try:
        assert is_rust_dispatch_available() is False
    finally:
        os.environ.pop("SKY_USE_RUST_DISPATCH", None)
        reset_rust_availability_cache()


def test_rust_input_adapter_lifecycle() -> None:
    if not hasattr(sky_player_rs, "RustInputBackend"):
        pytest.skip("RustInputBackend is diagnostic-backend feature gated")
    adapter = RustInputAdapter(mock=True)

    health = adapter.get_health()
    assert health.active_count == 0

    res_down = adapter.key_down((0x15, 0x16))
    assert res_down.success is True
    assert res_down.sent == (0x15, 0x16)

    health = adapter.get_health()
    assert health.active_count == 2

    res_up = adapter.key_up((0x15,))
    assert res_up.success is True
    assert res_up.sent == (0x15,)

    health = adapter.get_health()
    assert health.active_count == 1

    rel_out = adapter.release_all()
    assert rel_out.released_successfully is True
    assert rel_out.attempted == (0x16,)

    health = adapter.get_health()
    assert health.active_count == 0

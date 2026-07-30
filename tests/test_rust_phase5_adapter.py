"""Tests for Phase 5: Python Adapter Bridge and feature flag."""

from __future__ import annotations

import os

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
    adapter = RustInputAdapter(mock=True)

    health = adapter.get_health()
    assert health.active_count == 0

    res_down = adapter.key_down((2, 3))
    assert res_down.success is True
    assert res_down.sent == (2, 3)

    health = adapter.get_health()
    assert health.active_count == 2

    res_up = adapter.key_up((2,))
    assert res_up.success is True
    assert res_up.sent == (2,)

    health = adapter.get_health()
    assert health.active_count == 1

    rel_out = adapter.release_all()
    assert rel_out.released_successfully is True
    assert rel_out.attempted == (3,)

    health = adapter.get_health()
    assert health.active_count == 0

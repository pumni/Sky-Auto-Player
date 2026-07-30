"""Tests for Phase 3: RustInputBackend PyO3 wrapper and SendInput tracking."""

from __future__ import annotations

from typing import Any, cast

import pytest
import sky_player_rs  # type: ignore[import-not-found,import-untyped]

PHYSICAL_SCAN_CODES_PLUS_ONE = [
    2,
    3,
    4,
    5,
    6,
    16,
    17,
    18,
    19,
    20,
    30,
    31,
    32,
    33,
    34,
    35,
]


def test_rust_input_backend_basic_lifecycle() -> None:
    backend = sky_player_rs.RustInputBackend(mock=True)  # type: ignore[attr-defined]

    health = cast(dict[str, Any], backend.get_health())
    assert health["active_count"] == 0
    assert health["possibly_active_count"] == 0

    # Key down
    res_down = cast(dict[str, Any], backend.key_down([2, 3]))
    assert res_down["success"] is True
    assert res_down["sent"] == [2, 3]
    assert res_down["skipped_duplicates"] == []

    health = cast(dict[str, Any], backend.get_health())
    assert health["active_count"] == 2

    # Duplicate key down
    res_dup = cast(dict[str, Any], backend.key_down([2]))
    assert res_dup["success"] is True
    assert res_dup["sent"] == []
    assert res_dup["skipped_duplicates"] == [2]

    # Key up 1
    res_up = cast(dict[str, Any], backend.key_up([2]))
    assert res_up["success"] is True
    assert res_up["sent"] == [2]
    assert res_up["skipped_duplicates"] == []

    health = cast(dict[str, Any], backend.get_health())
    assert health["active_count"] == 1

    # Release all
    rel_out = cast(dict[str, Any], backend.release_all())
    assert rel_out["released_successfully"] is True
    assert rel_out["attempted"] == [3]

    health = cast(dict[str, Any], backend.get_health())
    assert health["active_count"] == 0


def test_rust_input_backend_full_instrument_release() -> None:
    backend = sky_player_rs.RustInputBackend(mock=True)  # type: ignore[attr-defined]
    backend.key_down([2, 16])

    rel_out = cast(dict[str, Any], backend.release_all_full_instrument())
    assert rel_out["released_successfully"] is True
    assert rel_out["attempted"] == [2, 16]

    health = cast(dict[str, Any], backend.get_health())
    assert health["active_count"] == 0


@pytest.mark.parametrize(
    "scan_codes",
    [
        [],
        [True],
        [999],
        [2, 2],
        PHYSICAL_SCAN_CODES_PLUS_ONE,
    ],
)
def test_rust_input_backend_rejects_untrusted_scan_code_batches(
    scan_codes: list[object],
) -> None:
    backend = sky_player_rs.RustInputBackend(mock=True)  # type: ignore[attr-defined]

    with pytest.raises((TypeError, ValueError)):
        backend.key_down(scan_codes)

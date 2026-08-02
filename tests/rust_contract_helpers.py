"""Shared assertions for the native dispatch correctness contract."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any


def assert_clean_finished(snapshot: Mapping[str, Any]) -> None:
    """Require every observable success-contract field to be clean.

    This intentionally does not accept an alternate terminal outcome.  A
    caller that expects ``quit``, ``skipped``, or ``error`` must assert that
    outcome directly instead of using this helper.
    """

    assert snapshot["status"] == "finished", snapshot
    assert snapshot["outcome"] == "finished", snapshot

    counts = snapshot["generation_status_counts"]
    generation_count = snapshot["generation_count"]
    assert counts["released"] == generation_count, snapshot
    assert counts["scheduled"] == 0, snapshot
    assert counts["active"] == 0, snapshot
    assert counts["release_pending"] == 0, snapshot
    assert counts["dropped_backend"] == 0, snapshot
    assert counts["dropped_conflict"] == 0, snapshot
    assert counts["dropped_expired"] == 0, snapshot
    assert counts["cancelled"] == 0, snapshot

    assert snapshot["active_count"] == 0, snapshot
    assert snapshot["possibly_active_count"] == 0, snapshot
    assert snapshot["failed_release_count"] == 0, snapshot
    assert snapshot["keys_dropped"] == 0, snapshot
    assert snapshot["chord_split_events"] == 0, snapshot
    assert snapshot["sendinput_partial_events"] == 0, snapshot
    assert snapshot["sendinput_zero_progress_failures"] == 0, snapshot
    assert snapshot["authored_keys_rejected"] == 0, snapshot

    release = snapshot["release_outcome"]
    assert release is not None, snapshot
    assert release["released_successfully"] is True, snapshot

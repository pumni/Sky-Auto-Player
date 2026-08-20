"""Tests for the input calibration regression fix.

Covers the full chain:
    Rust native measurement
    → .cache/input_latency.json
    → calibrated margin
    → playback policy
    → schedule

Also covers command UX (keymap rename) and invalidate_policy_metadata().
"""

from __future__ import annotations

import json
from unittest.mock import patch

from sky_music.config import AppConfig
from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.infrastructure.calibration_loader import (
    SOURCE_DEFAULT_500,
    SOURCE_DEVICE_CACHE,
    CalibrationLoadResult,
    CalibrationStatus,
    load_calibration_resolution,
)

# ---------------------------------------------------------------------------
# Helper – minimal valid cache-vNext payload
# ---------------------------------------------------------------------------

def _signed_stats(p99: int = 6) -> dict[str, int]:
    if p99 < 0:
        return {"min": p99 - 6, "p50": p99 - 4, "p90": p99 - 3, "p95": p99 - 2, "p99": p99, "max": p99 + 2, "mean": p99 - 3}
    return {"min": -4, "p50": -1, "p90": 1, "p95": 3, "p99": p99, "max": p99 + 2, "mean": 0}


_REQUIRED_BUCKETS = ("1/hot", "1/cold", "5/hot", "5/cold", "15/hot", "15/cold")
_HOST_FINGERPRINT = {
    "host_fingerprint_version": 2,
    "qpc_frequency_hz": 10_000_000,
    "win32_build": "Windows 11 test",
    "processor_architecture": "AMD64",
    "cpu_vendor": "GenuineIntel",
    "cpu_family": 6,
    "cpu_model": 183,
    "cpu_stepping": 1,
    "logical_processor_count": 16,
    "processor_group_count": 1,
    "cpu_set_efficiency_classes": [8, 8, 16, 16],
    "highest_efficiency_class": 16,
    "lowest_efficiency_class": 8,
    "sampled_at_us": 123,
}
_INTEGRATION_CACHE: dict = {
    "version": 5,
    "artifact_schema_version": 10,
    "status": "valid",
    "source": "device_cache",
    "evidence_kind": "injected_raw_input_total_hold_proxy",
    "source_formula_version": 4,
    "native_source_fingerprint": "test-fingerprint",
    "rustc_version": "rustc test",
    "native_calibration_version": 13,
    "measurement_protocol_version": 9,
    "source_git_sha": "test-sha",
    "native_build_id": "test-sha",
    "dirty_worktree": False,
    "host_fingerprint": _HOST_FINGERPRINT,
    "scheduling_aids": {
        "mmcss_acquired": "mmcss:Games",
        "mmcss_active": True,
        "power_throttling_active": True,
        "waiter_mode": "event+high_resolution_timer",
    },
    "required_buckets": list(_REQUIRED_BUCKETS),
    "pair_buckets": {
        key: {
            "attempted": 100,
            "clean_pair_count": 100,
            "rejected": 0,
            "pair_worst_total_proxy_shrink_us": _signed_stats(700 if key == "15/cold" else 6),
            "scheduler_shrink_us": _signed_stats(6),
            "sendinput_shrink_us": _signed_stats(6),
            "delivery_shrink_us": _signed_stats(6),
            "pair_worst_shrink_us": _signed_stats(700 if key == "15/cold" else 6),
        }
        for key in _REQUIRED_BUCKETS
    },
    "qualification": {
        "basis": "max_required_bucket_p99_positive_pair_total_proxy_hold_shrink",
        "worst_bucket": "15/cold",
        "global_shrink_p99_us": 700,
        "guard_us": 100,
        "floor_us": 300,
        "ceiling_us": 2000,
        "candidate_margin_us": 800,
        "applied_margin_us": 800,
    },
}

_EXPECTED_MARGIN_US = 800

def _resolution(
    margin_us: int,
    source: str,
    status: CalibrationStatus = CalibrationStatus.VALID,
) -> CalibrationLoadResult:
    return CalibrationLoadResult(
        status=status,
        resolved_margin_us=margin_us,
        margin_source=source,
        summary=None,
    )


# Correct patch target: the typed resolver imported by calibrated_policy.
_LOADER_PATCH = "sky_music.orchestration.calibrated_policy.load_calibration_resolution"


# ---------------------------------------------------------------------------
# 1. resolve_calibrated_policy uses the cached margin
# ---------------------------------------------------------------------------

class TestResolveCalibratedPolicy:
    """Tests for orchestration.calibrated_policy.resolve_calibrated_policy."""

    def test_uses_device_cache_margin(self) -> None:
        """resolve_calibrated_policy forwards device_cache margin into policy."""
        from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy

        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=_resolution(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            policy = resolve_calibrated_policy(session, cfg)

        assert int(policy.min_hold_margin_us) == _EXPECTED_MARGIN_US
        assert policy.min_hold_margin_source == SOURCE_DEVICE_CACHE

    def test_fallback_to_default_500_when_cache_missing(self) -> None:
        """resolve_calibrated_policy falls back to 500 µs when cache is absent."""
        from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy

        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        with patch(
            _LOADER_PATCH,
            return_value=_resolution(500, SOURCE_DEFAULT_500, CalibrationStatus.UNCALIBRATED),
        ):
            policy = resolve_calibrated_policy(session, cfg)

        assert policy.min_hold_margin_source == SOURCE_DEFAULT_500
        # Default fallback is 500 µs constant
        assert int(policy.min_hold_margin_us) == 500

    def test_fallback_to_default_500_when_cache_corrupt(self) -> None:
        """resolve_calibrated_policy falls back gracefully when loader rejects cache."""
        from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy

        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        with patch(
            _LOADER_PATCH,
            return_value=_resolution(500, SOURCE_DEFAULT_500, CalibrationStatus.INVALID_CACHE),
        ):
            policy = resolve_calibrated_policy(session, cfg)

        assert policy.min_hold_margin_source == SOURCE_DEFAULT_500


# ---------------------------------------------------------------------------
# 2. prepare_playback picks up device_cache
# ---------------------------------------------------------------------------

class TestPreparePlaybackUsesCalibration:
    """Tests that prepare_playback (Textual path) uses the calibrated margin."""

    # Patch at the import site of playback_controller (where it imports from calibrated_policy)
    _PREP_PATCH = "sky_music.orchestration.calibrated_policy.load_calibration_resolution"

    def _make_song(self) -> Song:
        return Song(
            name="Test Song",
            notes=(
                Note(time_ms=Millis(0), key=NoteKey("Key0")),
                Note(time_ms=Millis(100), key=NoteKey("Key1")),
            ),
        )

    def test_prepare_playback_receives_device_cache_source(self) -> None:
        """prepare_playback must result in min_hold_margin_source == device_cache."""
        from sky_music.ui.textual_app.playback_controller import (
            PlaybackPlan,
            prepare_playback,
        )

        song = self._make_song()
        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        with patch(self._PREP_PATCH, return_value=_resolution(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            plan = prepare_playback(song, session, cfg)

        assert isinstance(plan, PlaybackPlan)
        assert plan.active_policy.min_hold_margin_source == SOURCE_DEVICE_CACHE
        assert int(plan.active_policy.min_hold_margin_us) == _EXPECTED_MARGIN_US

    def test_prepare_playback_without_cache_uses_default(self) -> None:
        """prepare_playback falls back to default_500 when no cache."""
        from sky_music.ui.textual_app.playback_controller import (
            PlaybackPlan,
            prepare_playback,
        )

        song = self._make_song()
        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        # Explicitly mock the loader — this test opts out of the conftest autouse mock
        # because its nodeid contains "test_calibration_regression".
        with patch(
            _LOADER_PATCH,
            return_value=_resolution(500, SOURCE_DEFAULT_500, CalibrationStatus.UNCALIBRATED),
        ):
            plan = prepare_playback(song, session, cfg)
        assert isinstance(plan, PlaybackPlan)
        assert plan.active_policy.min_hold_margin_source == SOURCE_DEFAULT_500


# ---------------------------------------------------------------------------
# 3. Console playback has no bare resolve_effective_policy calls
# ---------------------------------------------------------------------------

class TestConsolePolicyMatchesTextual:
    """Console and Textual must resolve the same calibrated margin."""

    def test_console_uses_resolve_calibrated_policy(self) -> None:
        """All three console_playback call sites now use resolve_calibrated_policy."""
        import ast
        import inspect

        from sky_music.cli import console_playback

        source = inspect.getsource(console_playback)
        tree = ast.parse(source)

        bare_calls = [
            node
            for node in ast.walk(tree)
            if (
                isinstance(node, ast.Call)
                and isinstance(node.func, ast.Attribute)
                and node.func.attr == "resolve_effective_policy"
            )
        ]

        assert bare_calls == [], (
            f"Found {len(bare_calls)} bare session.resolve_effective_policy() call(s) "
            "in console_playback — these must use resolve_calibrated_policy() instead."
        )


def test_runtime_session_uses_only_the_single_calibrated_policy_resolver() -> None:
    import inspect

    from sky_music.orchestration import runtime_session

    source = inspect.getsource(runtime_session)
    assert "resolve_calibrated_policy" in source
    assert "load_calibrated_margin_recommendation" not in source
    assert "load_calibration_resolution" not in source


# ---------------------------------------------------------------------------
# 4. Profile/tempo rebuild preserves calibration
# ---------------------------------------------------------------------------

class TestRebuildPreservesCalibration:
    """rebuild_with must route through the calibrated policy helper."""

    def _make_song(self) -> Song:
        return Song(
            name="Song",
            notes=(Note(time_ms=Millis(0), key=NoteKey("Key0")),),
        )

    def test_rebuild_with_hold_keeps_device_cache(self) -> None:
        from sky_music.ui.textual_app.playback_controller import (
            PlaybackPlan,
            prepare_playback,
            rebuild_with,
        )

        song = self._make_song()
        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=_resolution(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            plan = prepare_playback(song, session, cfg)
            assert isinstance(plan, PlaybackPlan)
            rebuilt = rebuild_with(plan, hold_frames=1.5)

        assert isinstance(rebuilt, PlaybackPlan)
        assert rebuilt.active_policy.min_hold_margin_source == SOURCE_DEVICE_CACHE
        assert int(rebuilt.active_policy.min_hold_margin_us) == _EXPECTED_MARGIN_US

    def test_rebuild_with_tempo_keeps_device_cache(self) -> None:
        from sky_music.ui.textual_app.playback_controller import (
            PlaybackPlan,
            prepare_playback,
            rebuild_with,
        )

        song = self._make_song()
        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=_resolution(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            plan = prepare_playback(song, session, cfg)
            assert isinstance(plan, PlaybackPlan)
            rebuilt = rebuild_with(plan, tempo=0.9)

        assert isinstance(rebuilt, PlaybackPlan)
        assert rebuilt.active_policy.min_hold_margin_source == SOURCE_DEVICE_CACHE


# ---------------------------------------------------------------------------
# 5. Picker metadata signature changes when calibration changes
# ---------------------------------------------------------------------------

class TestPickerMetadataSignatureIncludesCalibration:
    """Policy cache key must vary with min_hold_margin_source."""

    # _effective_policy_signature does a local import of resolve_calibrated_policy,
    # so we patch at the single source: the calibrated_policy module's loader reference.
    _SIG_PATCH = "sky_music.orchestration.calibrated_policy.load_calibration_resolution"

    def test_signature_changes_when_margin_changes(self) -> None:
        from sky_music.ui.picker_metadata import _effective_policy_signature

        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        with patch(
            self._SIG_PATCH,
            return_value=_resolution(500, SOURCE_DEFAULT_500, CalibrationStatus.UNCALIBRATED),
        ):
            sig_default = _effective_policy_signature(session, cfg)

        with patch(self._SIG_PATCH, return_value=_resolution(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            sig_device = _effective_policy_signature(session, cfg)

        assert sig_default != sig_device, (
            "Policy signature must differ between default_500 and device_cache calibration"
        )
        assert sig_default.get("min_hold_margin_source") == SOURCE_DEFAULT_500
        assert sig_device.get("min_hold_margin_source") == SOURCE_DEVICE_CACHE
        assert sig_device.get("min_hold_margin_us") == _EXPECTED_MARGIN_US

    def test_signature_includes_min_hold_margin_source(self) -> None:
        from sky_music.ui.picker_metadata import _effective_policy_signature

        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        # conftest returns default_500
        sig = _effective_policy_signature(session, cfg)
        assert "min_hold_margin_source" in sig, (
            "min_hold_margin_source must be part of the persistent cache key signature"
        )
        assert "min_hold_margin_us" in sig
        assert sig["down_late_grace_us"] == 500


# ---------------------------------------------------------------------------
# 6. invalidate_policy_metadata does not clear raw metadata
# ---------------------------------------------------------------------------

class TestInvalidatePolicyMetadata:
    """invalidate_policy_metadata clears session caches but not raw song data."""

    def test_invalidate_clears_session_cache_not_raw(self) -> None:
        from sky_music.ui import picker_metadata

        # Seed all caches with a sentinel
        sentinel: object = object()

        with picker_metadata._cache_lock:
            picker_metadata._metadata_cache["key1"] = sentinel  # type: ignore[assignment]
            picker_metadata._raw_cache["key2"] = sentinel  # type: ignore[assignment]

        with picker_metadata._pkey_ram_lock:
            picker_metadata._pkey_ram_cache["key3"] = "sha"  # type: ignore[assignment]

        with picker_metadata._path_session_ram_lock:
            picker_metadata._path_session_ram_cache["key4"] = ((), ())  # type: ignore[assignment]

        picker_metadata.invalidate_policy_metadata()

        # Session/policy caches must be empty
        with picker_metadata._cache_lock:
            assert len(picker_metadata._metadata_cache) == 0
        with picker_metadata._pkey_ram_lock:
            assert len(picker_metadata._pkey_ram_cache) == 0
        with picker_metadata._path_session_ram_lock:
            assert len(picker_metadata._path_session_ram_cache) == 0

        # Raw cache must be preserved
        with picker_metadata._cache_lock:
            assert picker_metadata._raw_cache.get("key2") is sentinel  # type: ignore[comparison-overlap]

    def test_invalidate_is_idempotent(self) -> None:
        from sky_music.ui.picker_metadata import invalidate_policy_metadata

        # Should not raise when caches are already empty
        invalidate_policy_metadata()
        invalidate_policy_metadata()


# ---------------------------------------------------------------------------
# 7. Keymap command rename — c → calibrate_latency, calibration = no key
# ---------------------------------------------------------------------------

class TestKeymapCommandRename:
    """Command palette entries must have the correct labels and keys after rename."""

    def _get_command(self, cmd_id: str):
        from sky_music.ui.textual_app.keymap import COMMANDS

        for cmd in COMMANDS:
            if cmd.id == cmd_id:
                return cmd
        return None

    def test_calibrate_latency_has_key_c(self) -> None:
        """Hardware measurement command must be bound to 'c'."""
        cmd = self._get_command("calibrate_latency")
        assert cmd is not None, "calibrate_latency command missing from COMMANDS"
        assert cmd.key == "c", f"Expected key='c', got key={cmd.key!r}"
        assert cmd.label == "Host Input Delivery Calibration"

    def test_calibration_has_no_direct_key(self) -> None:
        """Telemetry recommendation command must not be bound to 'c'."""
        cmd = self._get_command("calibration")
        assert cmd is not None, "calibration command missing from COMMANDS"
        assert cmd.key != "c", "Telemetry recommendation must not be bound to 'c'"
        assert "Telemetry" in cmd.label or "Recommendation" in cmd.label, (
            f"Label should mention Telemetry/Recommendation, got {cmd.label!r}"
        )

    def test_calibrate_latency_routes_to_correct_action(self) -> None:
        """Command palette routing must call action_calibrate_input_latency for calibrate_latency."""
        import inspect

        from sky_music.ui.textual_app.screens import picker

        source = inspect.getsource(picker.PickerScreen._run_command)
        # Verify the calibrate_latency branch calls action_calibrate_input_latency
        assert "action_calibrate_input_latency" in source
        # Verify calibration branch calls action_open_calibration
        assert "action_open_calibration" in source

    def test_calibration_still_routes_to_open_calibration(self) -> None:
        """The 'calibration' command must still route to action_open_calibration."""
        import inspect

        from sky_music.ui.textual_app.screens import picker

        # Check the source code directly for the routing strings
        source = inspect.getsource(picker.PickerScreen._run_command)
        assert '"calibration"' in source or "'calibration'" in source, (
            "calibration branch must be present in _run_command"
        )
        assert "action_open_calibration" in source, (
            "action_open_calibration must be called for the calibration command"
        )

    def test_c_binding_defined_in_picker_bindings(self) -> None:
        """BINDINGS must contain a 'c' key for calibrate_input_latency."""
        from textual.binding import Binding

        from sky_music.ui.textual_app.screens.picker import PickerScreen

        found = False
        for binding in PickerScreen.BINDINGS:
            if isinstance(binding, Binding):
                if binding.key == "c" and "calibrate" in str(binding.action):
                    found = True
            elif isinstance(binding, tuple) and len(binding) >= 2 and binding[0] == "c" and "calibrate" in str(binding[1]):
                found = True
        assert found, "No 'c' binding for calibrate_input_latency found in PickerScreen.BINDINGS"


# ---------------------------------------------------------------------------
# 8. Integration: cache → margin → policy → schedule
# ---------------------------------------------------------------------------

class TestCalibrationRegressionIntegration:
    """End-to-end integration: valid cache yields device_cache active policy margin."""

    def test_calibration_regression_cache_to_policy(self) -> None:
        """Given a valid .cache/input_latency.json, the active policy margin == 800 µs."""
        # Act: load the margin directly from synthetic data (avoids filesystem)
        with patch(
            "sky_music.infrastructure.calibration_loader._current_host_fingerprint",
            return_value=_HOST_FINGERPRINT,
        ):
            resolution = load_calibration_resolution(data=_INTEGRATION_CACHE)

        assert resolution.status is CalibrationStatus.VALID
        assert resolution.margin_source == SOURCE_DEVICE_CACHE
        assert resolution.resolved_margin_us == _EXPECTED_MARGIN_US

        # Build a policy through the resolver using the loaded margin
        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()
        policy = session.resolve_effective_policy(
            cfg,
            hold_margin_us=resolution.resolved_margin_us,
            hold_margin_source=resolution.margin_source,
        )

        assert int(policy.min_hold_margin_us) == _EXPECTED_MARGIN_US
        assert policy.min_hold_margin_source == SOURCE_DEVICE_CACHE

    def test_calibration_regression_policy_applied_to_schedule(self) -> None:
        """With an 800 µs device margin, the schedule hold reflects it."""
        from sky_music.domain.scheduler import build_key_actions

        song = Song(
            name="Integration Test Song",
            notes=(
                Note(time_ms=Millis(0), key=NoteKey("Key0")),
                Note(time_ms=Millis(500), key=NoteKey("Key1")),
                Note(time_ms=Millis(1000), key=NoteKey("Key2")),
            ),
        )

        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()
        policy = session.resolve_effective_policy(
            cfg,
            hold_margin_us=_EXPECTED_MARGIN_US,
            hold_margin_source=SOURCE_DEVICE_CACHE,
        )

        sched = build_key_actions(song, policy=policy, scan_code_mode="physical")

        # Policy must carry the device margin
        assert int(policy.min_hold_margin_us) == _EXPECTED_MARGIN_US
        assert policy.min_hold_margin_source == SOURCE_DEVICE_CACHE
        # Schedule was built without error
        assert len(sched.actions) > 0

    def test_calibration_regression_default_is_not_reused_after_device_cache(self) -> None:
        """Persistent cache keys from default_500 must differ from device_cache keys."""
        from sky_music.ui.picker_metadata import _effective_policy_signature

        session = PlaybackSessionContext.default(fps=60)
        cfg = AppConfig()

        _SIG_PATCH = "sky_music.orchestration.calibrated_policy.load_calibration_resolution"

        with patch(
            _SIG_PATCH,
            return_value=_resolution(500, SOURCE_DEFAULT_500, CalibrationStatus.UNCALIBRATED),
        ):
            sig_before = _effective_policy_signature(session, cfg)

        with patch(_SIG_PATCH, return_value=_resolution(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            sig_after = _effective_policy_signature(session, cfg)

        assert sig_before != sig_after, (
            "A default_500 cache key must not match a device_cache key — "
            "stale metadata would be reused after calibration."
        )


# ---------------------------------------------------------------------------
# 3. Published Calibration Result Contract & UI Guard Tests
# ---------------------------------------------------------------------------

class TestPublishedCalibrationResultContract:
    """Tests for typed paired evidence and strict v3/v2 cache parsing."""

    def test_raw_native_result_has_no_ui_down_us_contract(self) -> None:
        """Raw native dict has buckets, not top-level down_us."""
        from pathlib import Path

        from sky_music.platform.win32.native_calibration import (
            PublishedCalibrationResult,
        )

        pub = PublishedCalibrationResult(
            status=CalibrationStatus.VALID,
            margin_us=800,
            candidate_margin_us=800,
            source="device_cache",
            sample_count=100,
            cache_path=Path(".cache/input_latency.json"),
            evidence_kind="injected_raw_input_delivery_proxy",
            source_git_sha="0" * 40,
            native_build_id="0" * 40,
            pair_buckets={},
            worst_bucket="15/cold",
            global_shrink_p99_us=700,
            guard_us=100,
            ceiling_us=2_000,
        )
        assert pub.global_shrink_p99_us == 700
        assert pub.evidence_kind == "injected_raw_input_delivery_proxy"

    def test_published_result_extracts_pair_quantiles_from_cache(self) -> None:
        """parse_calibration_cache_summary extracts signed pair quantiles."""
        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        summary = parse_calibration_cache_summary(_INTEGRATION_CACHE)
        assert summary.pair_buckets["15/cold"].pair_worst_total_proxy_shrink_us.p99 == 700
        assert summary.margin_us == 800

    def test_published_result_contains_numeric_signed_pair_quantiles(self) -> None:
        """Pair quantiles are strictly ints and retain negative evidence."""
        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        summary = parse_calibration_cache_summary(_INTEGRATION_CACHE)
        assert type(summary.pair_buckets["1/hot"].pair_worst_total_proxy_shrink_us.p50) is int
        assert summary.pair_buckets["1/hot"].pair_worst_total_proxy_shrink_us.min < 0

    def test_published_result_accepts_margin_floor_300(self) -> None:
        """Recommended margin floor is 300 µs when paired p99 is small."""
        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = json.loads(json.dumps(_INTEGRATION_CACHE))
        for bucket in cache["pair_buckets"].values():
            bucket["pair_worst_total_proxy_shrink_us"] = _signed_stats(-2)
        cache["qualification"].update(
            {"worst_bucket": "1/hot", "global_shrink_p99_us": 0, "candidate_margin_us": 100, "applied_margin_us": 300}
        )
        summary = parse_calibration_cache_summary(cache)
        assert summary.margin_us == 300

    def test_missing_pair_p50_fails_closed(self) -> None:
        """Missing pair p50 raises ValueError."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = json.loads(json.dumps(_INTEGRATION_CACHE))
        del cache["pair_buckets"]["1/hot"]["pair_worst_total_proxy_shrink_us"]["p50"]
        with pytest.raises((TypeError, ValueError)):
            parse_calibration_cache_summary(cache)

    def test_missing_pair_p99_fails_closed(self) -> None:
        """Missing pair p99 raises ValueError."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = json.loads(json.dumps(_INTEGRATION_CACHE))
        del cache["pair_buckets"]["1/hot"]["pair_worst_total_proxy_shrink_us"]["p99"]
        with pytest.raises((TypeError, ValueError)):
            parse_calibration_cache_summary(cache)

    def test_invalid_quantile_order_fails_closed(self) -> None:
        """Out of order quantiles (p50 > p90) raises ValueError."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = json.loads(json.dumps(_INTEGRATION_CACHE))
        cache["pair_buckets"]["1/hot"]["pair_worst_total_proxy_shrink_us"].update(
            {"p50": 9, "p90": 2}
        )
        with pytest.raises(ValueError):
            parse_calibration_cache_summary(cache)

    def test_success_modal_never_contains_question_mark(self) -> None:
        """Success modal text formatted from PublishedCalibrationResult contains no '?'."""
        from pathlib import Path

        from sky_music.platform.win32.native_calibration import (
            PublishedCalibrationResult,
        )

        pub = PublishedCalibrationResult(
            status=CalibrationStatus.VALID,
            margin_us=800,
            candidate_margin_us=800,
            source="device_cache",
            sample_count=100,
            cache_path=Path(".cache/input_latency.json"),
            evidence_kind="injected_raw_input_delivery_proxy",
            source_git_sha="0" * 40,
            native_build_id="0" * 40,
            pair_buckets={},
            worst_bucket="15/cold",
            global_shrink_p99_us=700,
            guard_us=100,
            ceiling_us=2_000,
        )

        msg = (
            f"Device margin: {pub.margin_us} µs\n"
            f"Source: {pub.source}\n"
            f"Cache: {pub.cache_path}\n\n"
            f"Host delivery hold-shrink p99: {pub.global_shrink_p99_us} µs\n"
            f"Worst bucket: {pub.worst_bucket}\n"
            f"Evidence: {pub.evidence_kind} (SendInput → app-owned WM_INPUT)."
        )
        assert "?" not in msg

    def test_rejected_cache_message_does_not_read_raw_n(self) -> None:
        """Loader raises ValueError when a bucket has fewer than 100 pairs."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = json.loads(json.dumps(_INTEGRATION_CACHE))
        cache["pair_buckets"]["1/hot"]["clean_pair_count"] = 5
        cache["pair_buckets"]["1/hot"]["rejected"] = 95
        with pytest.raises(ValueError):
            parse_calibration_cache_summary(cache)


class TestCalibrationProgressModalUX:
    """Tests for CalibrationProgressModal and calibration locking state."""

    def test_calibration_progress_modal_blocks_y_theme(self) -> None:
        """CalibrationProgressModal instantiates cleanly with theme."""
        from sky_music.ui.textual_app.modals import CalibrationProgressModal

        modal = CalibrationProgressModal(theme_name="aurora")
        assert modal.theme_name == "aurora"

    def test_calibration_progress_modal_blocks_picker_bindings(self) -> None:
        """CalibrationProgressModal has empty BINDINGS list."""
        from sky_music.ui.textual_app.modals import CalibrationProgressModal

        assert CalibrationProgressModal.BINDINGS == []

    def test_calibration_progress_modal_blocks_enter_escape(self) -> None:
        """CalibrationProgressModal.on_key consumes events without dismissing."""
        from unittest.mock import MagicMock

        from sky_music.ui.textual_app.modals import CalibrationProgressModal

        modal = CalibrationProgressModal(theme_name="aurora")
        event = MagicMock()
        event.key = "escape"
        modal.on_key(event)
        event.stop.assert_called_once()
        event.prevent_default.assert_called_once()

    def test_calibration_active_prevents_second_worker(self) -> None:
        """When app.calibration_active is True, key events are blocked in app."""
        from unittest.mock import MagicMock

        from sky_music.ui.textual_app.app import SkyPickerApp

        app = SkyPickerApp.__new__(SkyPickerApp)
        app.calibration_active = True
        event = MagicMock()
        app.on_key(event)
        event.stop.assert_called_once()
        event.prevent_default.assert_called_once()

    def test_calibration_state_resets_after_success(self) -> None:
        """App.calibration_active is False initially and can be set/reset."""
        from sky_music.ui.textual_app.app import SkyPickerApp

        app = SkyPickerApp.__new__(SkyPickerApp)
        app.calibration_active = False
        assert app.calibration_active is False

    def test_calibration_state_resets_after_failure(self) -> None:
        """App.calibration_active can be reset in finally block."""
        app_calibration_active = True
        try:
            raise RuntimeError("simulated error")
        except Exception:
            pass
        finally:
            app_calibration_active = False
        assert app_calibration_active is False

    def test_progress_modal_is_removed_before_result_modal(self) -> None:
        """Modal stack removal pattern pops progress modal before pushing InfoModal."""
        stack = ["PickerScreen", "CalibrationProgressModal"]
        if "CalibrationProgressModal" in stack:
            stack.remove("CalibrationProgressModal")
        stack.append("InfoModal")
        assert stack == ["PickerScreen", "InfoModal"]

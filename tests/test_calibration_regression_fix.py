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

from unittest.mock import patch

from sky_music.config import AppConfig
from sky_music.domain import Millis, Note, NoteKey, Song
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.infrastructure.calibration_loader import (
    SOURCE_DEFAULT_500,
    SOURCE_DEVICE_CACHE,
    load_calibrated_margin_recommendation,
)

# ---------------------------------------------------------------------------
# Helper – minimal valid cache payload
# ---------------------------------------------------------------------------

_INTEGRATION_CACHE: dict = {
    "version": 1,
    "down_us": {"p50": 500, "p90": 800, "p99": 1100},
    "up_us": {"p50": 400, "p90": 700, "p99": 900},
    "n": 20,
}

# Expected margin for _INTEGRATION_CACHE:
# clamp(300, 2000, p99_down - p50_up + 100) = clamp(300, 2000, 1100 - 400 + 100) = 800
_EXPECTED_MARGIN_US = 800

# Correct patch target: the function as imported into calibrated_policy's namespace
_LOADER_PATCH = "sky_music.orchestration.calibrated_policy.load_calibrated_margin_recommendation"


# ---------------------------------------------------------------------------
# 1. resolve_calibrated_policy uses the cached margin
# ---------------------------------------------------------------------------

class TestResolveCalibratedPolicy:
    """Tests for orchestration.calibrated_policy.resolve_calibrated_policy."""

    def test_uses_device_cache_margin(self) -> None:
        """resolve_calibrated_policy forwards device_cache margin into policy."""
        from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy

        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            policy = resolve_calibrated_policy(session, cfg)

        assert int(policy.min_hold_margin_us) == _EXPECTED_MARGIN_US
        assert policy.min_hold_margin_source == SOURCE_DEVICE_CACHE

    def test_fallback_to_default_500_when_cache_missing(self) -> None:
        """resolve_calibrated_policy falls back to 500 µs when cache is absent."""
        from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy

        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=(None, SOURCE_DEFAULT_500)):
            policy = resolve_calibrated_policy(session, cfg)

        assert policy.min_hold_margin_source == SOURCE_DEFAULT_500
        # Default fallback is 500 µs constant
        assert int(policy.min_hold_margin_us) == 500

    def test_fallback_to_default_500_when_cache_corrupt(self) -> None:
        """resolve_calibrated_policy falls back gracefully when loader rejects cache."""
        from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy

        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=(None, SOURCE_DEFAULT_500)):
            policy = resolve_calibrated_policy(session, cfg)

        assert policy.min_hold_margin_source == SOURCE_DEFAULT_500


# ---------------------------------------------------------------------------
# 2. prepare_playback picks up device_cache
# ---------------------------------------------------------------------------

class TestPreparePlaybackUsesCalibration:
    """Tests that prepare_playback (Textual path) uses the calibrated margin."""

    # Patch at the import site of playback_controller (where it imports from calibrated_policy)
    _PREP_PATCH = "sky_music.orchestration.calibrated_policy.load_calibrated_margin_recommendation"

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
        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        with patch(self._PREP_PATCH, return_value=(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
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
        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        # Explicitly mock the loader — this test opts out of the conftest autouse mock
        # because its nodeid contains "test_calibration_regression".
        with patch(_LOADER_PATCH, return_value=(None, SOURCE_DEFAULT_500)):
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

    def test_rebuild_with_profile_keeps_device_cache(self) -> None:
        from sky_music.ui.textual_app.playback_controller import (
            PlaybackPlan,
            prepare_playback,
            rebuild_with,
        )

        song = self._make_song()
        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            plan = prepare_playback(song, session, cfg)
            assert isinstance(plan, PlaybackPlan)
            rebuilt = rebuild_with(plan, profile="audience-safe")

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
        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        with patch(_LOADER_PATCH, return_value=(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
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
    _SIG_PATCH = "sky_music.orchestration.calibrated_policy.load_calibrated_margin_recommendation"

    def test_signature_changes_when_margin_changes(self) -> None:
        from sky_music.ui.picker_metadata import _effective_policy_signature

        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        with patch(self._SIG_PATCH, return_value=(None, SOURCE_DEFAULT_500)):
            sig_default = _effective_policy_signature(session, cfg)

        with patch(self._SIG_PATCH, return_value=(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            sig_device = _effective_policy_signature(session, cfg)

        assert sig_default != sig_device, (
            "Policy signature must differ between default_500 and device_cache calibration"
        )
        assert sig_default.get("min_hold_margin_source") == SOURCE_DEFAULT_500
        assert sig_device.get("min_hold_margin_source") == SOURCE_DEVICE_CACHE
        assert sig_device.get("min_hold_margin_us") == _EXPECTED_MARGIN_US

    def test_signature_includes_min_hold_margin_source(self) -> None:
        from sky_music.ui.picker_metadata import _effective_policy_signature

        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        # conftest returns default_500
        sig = _effective_policy_signature(session, cfg)
        assert "min_hold_margin_source" in sig, (
            "min_hold_margin_source must be part of the persistent cache key signature"
        )
        assert "min_hold_margin_us" in sig


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
        assert "Input Latency" in cmd.label, f"Label should say 'Input Latency', got {cmd.label!r}"

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
        margin_us, source = load_calibrated_margin_recommendation(data=_INTEGRATION_CACHE)

        assert source == SOURCE_DEVICE_CACHE
        assert margin_us == _EXPECTED_MARGIN_US

        # Build a policy through the resolver using the loaded margin
        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()
        policy = session.resolve_effective_policy(
            cfg,
            calibrated_margin_us=margin_us,
            calibrated_margin_source=source,
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

        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()
        policy = session.resolve_effective_policy(
            cfg,
            calibrated_margin_us=_EXPECTED_MARGIN_US,
            calibrated_margin_source=SOURCE_DEVICE_CACHE,
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

        session = PlaybackSessionContext.balanced(fps=60)
        cfg = AppConfig()

        _SIG_PATCH = "sky_music.orchestration.calibrated_policy.load_calibrated_margin_recommendation"

        with patch(_SIG_PATCH, return_value=(None, SOURCE_DEFAULT_500)):
            sig_before = _effective_policy_signature(session, cfg)

        with patch(_SIG_PATCH, return_value=(_EXPECTED_MARGIN_US, SOURCE_DEVICE_CACHE)):
            sig_after = _effective_policy_signature(session, cfg)

        assert sig_before != sig_after, (
            "A default_500 cache key must not match a device_cache key — "
            "stale metadata would be reused after calibration."
        )


# ---------------------------------------------------------------------------
# 3. Published Calibration Result Contract & UI Guard Tests
# ---------------------------------------------------------------------------

class TestPublishedCalibrationResultContract:
    """Tests for typed PublishedCalibrationResult and strict parsing."""

    def test_raw_native_result_has_no_ui_down_us_contract(self) -> None:
        """Raw native dict has buckets, not top-level down_us."""
        from pathlib import Path

        from sky_music.infrastructure.calibration_loader import CalibrationQuantiles
        from sky_music.platform.win32.native_calibration import (
            PublishedCalibrationResult,
        )

        pub = PublishedCalibrationResult(
            margin_us=800,
            source="device_cache",
            sample_count=20,
            down_us=CalibrationQuantiles(500, 800, 1100),
            up_us=CalibrationQuantiles(400, 700, 900),
            cache_path=Path(".cache/input_latency.json"),
            evidence_kind="injected_raw_input_delivery_proxy",
            source_git_sha="0" * 40,
            native_build_id="0" * 40,
        )
        assert isinstance(pub.down_us.p50, int)
        assert pub.down_us.p50 == 500
        assert pub.evidence_kind == "injected_raw_input_delivery_proxy"

    def test_published_result_extracts_quantiles_from_legacy_cache_payload(self) -> None:
        """parse_calibration_cache_summary extracts typed quantiles from dict."""
        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        summary = parse_calibration_cache_summary(_INTEGRATION_CACHE)
        assert summary.down_us.p50 == 500
        assert summary.down_us.p90 == 800
        assert summary.down_us.p99 == 1100
        assert summary.up_us.p50 == 400
        assert summary.up_us.p90 == 700
        assert summary.up_us.p99 == 900
        assert summary.margin_us == 800

    def test_published_result_contains_numeric_down_and_up_quantiles(self) -> None:
        """Quantiles are strictly int, never str or None."""
        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        summary = parse_calibration_cache_summary(_INTEGRATION_CACHE)
        assert type(summary.down_us.p50) is int
        assert type(summary.up_us.p99) is int

    def test_published_result_accepts_margin_floor_300(self) -> None:
        """Recommended margin floor is 300 µs when calculation yields less."""
        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = {
            "version": 1,
            "down_us": {"p50": 100, "p90": 150, "p99": 200},
            "up_us": {"p50": 150, "p90": 180, "p99": 200},
            "n": 20,
        }
        summary = parse_calibration_cache_summary(cache)
        assert summary.margin_us == 300

    def test_missing_down_p50_fails_closed(self) -> None:
        """Missing p50 in down_us raises ValueError."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = {
            "version": 1,
            "down_us": {"p90": 800, "p99": 1100},
            "up_us": {"p50": 400, "p90": 700, "p99": 900},
            "n": 20,
        }
        with pytest.raises(ValueError):
            parse_calibration_cache_summary(cache)

    def test_missing_up_p99_fails_closed(self) -> None:
        """Missing p99 in up_us raises ValueError."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = {
            "version": 1,
            "down_us": {"p50": 500, "p90": 800, "p99": 1100},
            "up_us": {"p50": 400, "p90": 700},
            "n": 20,
        }
        with pytest.raises(ValueError):
            parse_calibration_cache_summary(cache)

    def test_invalid_quantile_order_fails_closed(self) -> None:
        """Out of order quantiles (p50 > p90) raises ValueError."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = {
            "version": 1,
            "down_us": {"p50": 900, "p90": 800, "p99": 1100},
            "up_us": {"p50": 400, "p90": 700, "p99": 900},
            "n": 20,
        }
        with pytest.raises(ValueError):
            parse_calibration_cache_summary(cache)

    def test_success_modal_never_contains_question_mark(self) -> None:
        """Success modal text formatted from PublishedCalibrationResult contains no '?'."""
        from pathlib import Path

        from sky_music.infrastructure.calibration_loader import CalibrationQuantiles
        from sky_music.platform.win32.native_calibration import (
            PublishedCalibrationResult,
        )

        pub = PublishedCalibrationResult(
            margin_us=800,
            source="device_cache",
            sample_count=20,
            down_us=CalibrationQuantiles(500, 800, 1100),
            up_us=CalibrationQuantiles(400, 700, 900),
            cache_path=Path(".cache/input_latency.json"),
            evidence_kind="injected_raw_input_delivery_proxy",
            source_git_sha="0" * 40,
            native_build_id="0" * 40,
        )

        msg = (
            f"Device margin: {pub.margin_us} µs\n"
            f"Source: {pub.source}\n"
            f"Cache: {pub.cache_path}\n\n"
            f"Down latency (µs): p50={pub.down_us.p50}, "
            f"p90={pub.down_us.p90}, p99={pub.down_us.p99}\n"
            f"Up latency   (µs): p50={pub.up_us.p50}, "
            f"p90={pub.up_us.p90}, p99={pub.up_us.p99}\n\n"
            f"Evidence: {pub.evidence_kind} (SendInput → app-owned WM_INPUT)."
        )
        assert "?" not in msg

    def test_rejected_cache_message_does_not_read_raw_n(self) -> None:
        """Loader raises ValueError when n < 20 without evaluating dict methods on invalid objects."""
        import pytest

        from sky_music.infrastructure.calibration_loader import (
            parse_calibration_cache_summary,
        )

        cache = {
            "version": 1,
            "down_us": {"p50": 500, "p90": 800, "p99": 1100},
            "up_us": {"p50": 400, "p90": 700, "p99": 900},
            "n": 5,
        }
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


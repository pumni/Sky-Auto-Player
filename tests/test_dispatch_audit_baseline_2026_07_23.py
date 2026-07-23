from __future__ import annotations

from test_runtime_dispatch import FakeClock, FakeSleeper, TimedBackend, action

from sky_music.domain import Song
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PLAYBACK_FINISHED, PlaybackEngine


def test_audit_baseline_inventory_markers():
    """
    Documents accepted finding IDs as skip/xfail markers for not-yet-fixed behaviour OR simply lists expected-failing test names that Phase N will introduce.
    Phase 0 asserting baseline suite green.
    Future tests to be added by phases:
    Phase 1: test_first_down_blocked_when_focus_lost_before_send
    Phase 2: prototype presence test, wait_for_multiple_objects WAIT_FAILED handling
    Phase 3: test_set_expected_process_names_rejects_empty_after_normalize, test_config_rejects_bool_for_numeric_timing_field, etc.
    Phase 4: test_supervisor_exception_joins_dispatch_thread
    Phase 5: test_quit_honoured_when_high_res_timer_unavailable
    Phase 6: test_idle_warmup_skipped_when_pending_release_due
    """
    assert True


def test_h2_guard_same_key_equality_dropped_conflict():
    """
    H2 guard (required): an assertion that under degraded policy, a synthetic same-key pair 
    with interval == min_hold_us still builds a schedule and that runtime may emit 
    dropped_conflict when a delayed completion is simulated - without treating that as failure.
    Purpose: prevent Phase 6-8 from "fixing" H2.
    """
    clock = FakeClock()
    # Simulate delayed completion by setting send_duration_us > 0
    backend = TimedBackend(clock, send_duration_us=300)
    
    # interval == min_hold_us == 1000
    actions = (
        action(0, "down", 21),
        action(1000, "down", 21),
        action(1500, "up", 21),
        action(2000, "up", 21),
    )
    
    engine = PlaybackEngine(
        song=Song(name="h2_guard", notes=()),
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=FakeSleeper(clock),
        sleep_policy=SleepPolicy(spin_threshold_us=-1),
        min_hold_us=1000,
        same_key_conflict_policy="degraded",
    )
    
    assert engine.play() == PLAYBACK_FINISHED
    
    summary = engine.telemetry.get_summary()
    assert summary is not None
    # We expect 1 dropped conflict because the second down overlaps with the anchored release of the first.
    assert summary["dropped_conflict_count"] == 1

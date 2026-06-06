"""Observe-only sender-warmup instrumentation flows into telemetry records and summary."""

from __future__ import annotations

from sky_music.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import DryRunBackend
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PlaybackEngine


def _action(at_us: int, kind: str, scan: int) -> KeyAction:
    return KeyAction(
        kind=kind,  # type: ignore[arg-type]
        scan_codes=(ScanCode(scan),),
        at_us=Microseconds(at_us),
        reason="warmup-test",
    )


def _play_with_telemetry() -> PlaybackEngine:
    engine = PlaybackEngine(
        song=Song(name="warmup", notes=()),
        # A spaced gap before the 2nd onset guarantees a non-trivial idle_gap on it.
        actions=(
            _action(0, "down", 21),
            _action(2_000, "up", 21),
            _action(40_000, "down", 22),
            _action(42_000, "up", 22),
        ),
        backend=DryRunBackend(),
        telemetry_enabled=True,
        require_focus=False,
        sleep_policy=SleepPolicy(spin_threshold_us=500),
        use_dispatch_thread=True,
    )
    engine.play()
    return engine


def test_records_carry_warmup_columns() -> None:
    engine = _play_with_telemetry()
    sent = [r for r in engine.telemetry.records if r.get("sent_scan_codes")]
    assert sent, "expected at least one sent dispatch"
    for r in sent:
        assert "idle_gap_us" in r and "pre_send_spin_us" in r
        assert isinstance(r["idle_gap_us"], int) and r["idle_gap_us"] >= 0
        assert isinstance(r["pre_send_spin_us"], int) and r["pre_send_spin_us"] >= 0


def test_second_onset_after_gap_has_idle_gap() -> None:
    engine = _play_with_telemetry()
    # The down at 40ms follows a ~38ms wait, so its idle_gap must be clearly non-zero, while a
    # back-to-back send (the up right after a down) idles essentially not at all.
    downs = [r for r in engine.telemetry.records if r["kind"] == "down" and r.get("sent_scan_codes")]
    assert len(downs) >= 2
    assert downs[1]["idle_gap_us"] > 5_000


def test_summary_exposes_send_warmup_split() -> None:
    engine = _play_with_telemetry()
    summary = engine.telemetry.get_summary()
    assert summary is not None
    warmup = summary["send_warmup"]
    assert warmup["cold_threshold_us"] == 20_000
    assert warmup["cold_send_count"] + warmup["warm_send_count"] == len(
        [r for r in engine.telemetry.records if r.get("sent_scan_codes")]
    )
    for key in ("send_duration_cold_us", "send_duration_warm_us", "idle_gap_us", "pre_send_spin_us"):
        assert key in warmup and "p99_us" in warmup[key]

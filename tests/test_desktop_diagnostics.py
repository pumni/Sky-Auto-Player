from __future__ import annotations

from sky_music.orchestration.desktop_diagnostics import DesktopDiagnosticsService
from sky_music.orchestration.native_models import BackendHealth, ProgressCounters


def _counters() -> ProgressCounters:
    return ProgressCounters(2_500, 3, 1, 0, 900, 1, (100, 200, 500, 1_500))


def _backend() -> BackendHealth:
    return BackendHealth(
        active_count=2,
        possibly_active_count=2,
        failed_release_count=0,
        last_error=None,
        keys_dropped=1,
        chord_split_events=2,
    )


def test_diagnostics_is_disabled_and_rate_limited_by_default() -> None:
    published: list[tuple[str, dict[str, object]]] = []
    service = DesktopDiagnosticsService(
        publish_event=lambda name, payload: published.append((name, payload))
    )

    assert service.publish_progress(_counters(), _backend(), now=0.0) is False
    assert published == []

    assert service.set_enabled(True) is True
    assert service.publish_progress(_counters(), _backend(), now=0.0) is True
    assert service.publish_progress(_counters(), _backend(), now=0.05) is False
    assert service.publish_progress(_counters(), _backend(), now=0.1) is True
    assert [name for name, _ in published] == [
        "diagnostics.snapshot",
        "diagnostics.snapshot",
    ]
    assert published[0][1]["p50_ms"] == 0.5
    assert published[0][1]["p95_ms"] == 1.5
    assert published[0][1]["active_keys"] == 2
    assert published[0][1]["release_late_2ms"] == 1

    service.set_enabled(False)
    assert service.publish_progress(_counters(), _backend(), now=0.2) is False
    assert len(published) == 2


def test_diagnostics_reenable_resets_sampling_window_but_not_sequence() -> None:
    published: list[dict[str, object]] = []
    service = DesktopDiagnosticsService(
        publish_event=lambda _name, payload: published.append(payload)
    )
    service.set_enabled(True)
    service.publish_progress(_counters(), None, now=10.0)
    service.set_enabled(False)
    service.set_enabled(True)
    service.publish_progress(_counters(), None, now=10.0)

    assert [item["seq"] for item in published] == [1, 2]
    assert all(item["backend_status"] == "unavailable" for item in published)


def test_diagnostics_publish_callback_is_not_a_scheduler_input() -> None:
    published: list[dict[str, object]] = []
    service = DesktopDiagnosticsService(
        publish_event=lambda _name, payload: published.append(payload)
    )
    service.set_enabled(True)
    service.publish_progress(_counters(), _backend(), session_id="a" * 32, now=0.0)

    assert published[0]["session_id"] == "a" * 32
    assert published[0]["late_10ms"] == 0

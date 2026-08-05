from sky_music.orchestration.native_diagnostics import diagnose_native_playback


def test_diagnostics_prioritize_backend_rejection() -> None:
    result = diagnose_native_playback(
        {"chords_rejected": 1, "sendinput_path_degraded": True}
    )
    assert result.category == "mixed"
    assert "native backend rejection counters are non-zero" in result.evidence


def test_diagnostics_keep_latency_without_rejection() -> None:
    assert diagnose_native_playback({"wait_path_degraded": True}).category == (
        "scheduler_wake_latency"
    )
    assert diagnose_native_playback({"bookkeeping_degraded": True}).category == (
        "bookkeeping_latency"
    )


def test_diagnostics_label_hold_visibility_as_inference() -> None:
    result = diagnose_native_playback(
        {
            "game_fps": 60,
            "configured_hold_us": 16_667,
            "sendinput_path_degraded": True,
        }
    )
    assert result.category == "hold_visibility_risk"
    assert any("inference" in evidence for evidence in result.evidence)

from sky_music.orchestration.native_diagnostics import diagnose_native_playback


def test_diagnostics_prioritize_backend_rejection() -> None:
    result = diagnose_native_playback(
        {"chords_rejected": 1, "sendinput_path_degraded": True}
    )
    assert result.category == "backend_rejection"
    assert "native backend rejection counters are non-zero" in result.evidence


def test_diagnostics_keep_latency_without_rejection() -> None:
    assert diagnose_native_playback({"wait_path_degraded": True}).category == (
        "scheduler_wake_degraded"
    )
    assert diagnose_native_playback({"core_post_send_degraded": True}).category == (
        "post_send_degraded"
    )


def test_diagnostics_label_hold_visibility_as_inference() -> None:
    result = diagnose_native_playback(
        {
            "game_fps": 60,
            "configured_hold_us": 16_667,
            "sendinput_path_degraded": True,
        }
    )
    assert result.category == "send_latency_degraded"
    assert all("game" not in evidence.lower() for evidence in result.evidence)


def test_diagnostics_read_nested_generation_drop_and_saturation_vectors() -> None:
    result = diagnose_native_playback(
        {
            "generation_status_counts": {"released": 10, "dropped_backend": 1},
            "lead_saturation_count_down": [0, 0, 1],
            "lead_saturation_count_up": [0, 0],
        }
    )
    assert result.category == "backend_rejection"

    lead = diagnose_native_playback({"lead_saturation_count_down": [0, 0, 1]})
    assert lead.category == "lead_saturated"
    assert diagnose_native_playback({"lead_saturation_count_up": [0, 0]}).category == (
        "clean_native_delivery"
    )


def test_diagnostics_do_not_treat_bool_as_counter() -> None:
    result = diagnose_native_playback(
        {
            "keys_dropped": True,
            "lead_saturation_count_down": [False, False],
        }
    )
    assert result.category == "clean_native_delivery"


def test_recovered_partial_retry_is_not_final_backend_rejection() -> None:
    result = diagnose_native_playback(
        {"recovered_partial_up_retries": 1, "sendinput_path_degraded": False}
    )
    assert result.category == "clean_native_delivery"

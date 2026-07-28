from sky_music.platform.win32 import inputs


def test_prewarm_populates_cache_with_correct_flags(monkeypatch):
    inputs.reset_prewarm_diagnostics()
    inputs._ARRAY_CACHE.clear()
    monkeypatch.setattr(inputs.user32, "SendInput", lambda count, array, size: count)
    
    # Prewarm a single down key and a chord up key
    shapes = [((0x15,), False), ((0x15, 0x16), True)]
    inputs.prewarm_input_arrays(shapes)
    
    # Cache should have 2 entries
    assert len(inputs._ARRAY_CACHE) == 2
    
    down_flags = inputs.KEYEVENTF_SCANCODE
    up_flags = inputs.KEYEVENTF_SCANCODE | inputs.KEYEVENTF_KEYUP
    
    assert ((0x15,), down_flags) in inputs._ARRAY_CACHE
    assert ((0x15, 0x16), up_flags) in inputs._ARRAY_CACHE

    diagnostics = inputs.get_prewarm_diagnostics()
    assert diagnostics["unique_down_shape_count"] == 1
    assert diagnostics["unique_up_shape_count"] == 1
    assert diagnostics["total_input_slots"] == 3
    assert diagnostics["approx_payload_bytes"] == 3 * inputs._INPUT_SIZE
    duration_us = diagnostics["prewarm_duration_us"]
    frequency = diagnostics["shape_frequency"]
    assert isinstance(duration_us, int) and duration_us >= 0
    assert isinstance(frequency, dict)
    assert frequency["down:21"] == 1
    assert frequency["up:21,22"] == 1
    
    # Verify that calling send_scan_code_batch_impl directly doesn't increase cache size (cache hit)
    inputs._send_scan_code_batch_impl((0x15,), down_flags, complete_remainder=False)
    assert len(inputs._ARRAY_CACHE) == 2


def test_prewarm_diagnostics_record_lazy_miss_and_clear_outcome(monkeypatch):
    inputs.reset_prewarm_diagnostics()
    inputs._ARRAY_CACHE.clear()
    inputs._INPUT_CACHE.clear()

    monkeypatch.setattr(inputs.user32, "SendInput", lambda count, array, size: count)
    inputs.prewarm_input_arrays([((0x15,), False)])
    assert inputs.get_prewarm_diagnostics()["cache_miss_count"] == 0

    inputs._send_scan_code_batch_impl(
        (0x16,),
        inputs.KEYEVENTF_SCANCODE,
        complete_remainder=False,
    )
    diagnostics = inputs.get_prewarm_diagnostics()
    assert diagnostics["cache_miss_count"] == 1
    assert diagnostics["lazy_build_count"] == 1
    first_hit_us = diagnostics["first_hit_lazy_build_duration_us"]
    max_lazy_build_us = diagnostics["lazy_build_duration_us_max"]
    assert isinstance(first_hit_us, int) and first_hit_us >= 0
    assert isinstance(max_lazy_build_us, int) and max_lazy_build_us >= 0

    cleared = inputs.clear_array_cache()
    diagnostics = inputs.get_prewarm_diagnostics()
    assert cleared == 2
    assert diagnostics["last_clear_cache_entries"] == 2
    assert diagnostics["last_clear_cache_slots"] == 2

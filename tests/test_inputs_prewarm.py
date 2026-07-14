from sky_music.platform.win32 import inputs


def test_prewarm_populates_cache_with_correct_flags(monkeypatch):
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
    
    # Verify that calling send_scan_code_batch_impl directly doesn't increase cache size (cache hit)
    inputs._send_scan_code_batch_impl((0x15,), down_flags, complete_remainder=False)
    assert len(inputs._ARRAY_CACHE) == 2

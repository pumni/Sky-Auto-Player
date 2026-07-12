from sky_music.platform.win32 import inputs


def test_cached_key_input_has_sky_player_signature():
    # Test caching for both a common key and flags
    scan_code = 0x15
    flags = 0

    # Force a cache miss by using a unique combination if needed,
    # but reset_window_cache() or just calling it directly is fine
    # since we just patched the module.

    # We just fetch from the _cached_key_input
    cached_input = inputs._cached_key_input(scan_code, flags)

    assert cached_input.ki.wScan == scan_code
    assert cached_input.ki.dwFlags == flags
    assert cached_input.ki.dwExtraInfo == inputs.SKY_PLAYER_SIGNATURE


def test_signature_persists_across_cache_hits():
    scan_code = 0x16
    flags = inputs.KEYEVENTF_KEYUP

    cached1 = inputs._cached_key_input(scan_code, flags)
    cached2 = inputs._cached_key_input(scan_code, flags)

    assert cached1 is cached2
    assert cached2.ki.dwExtraInfo == inputs.SKY_PLAYER_SIGNATURE

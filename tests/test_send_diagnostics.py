"""Partial-send diagnostics: confirm a chord that does not inject atomically is counted.

This is the instrumentation behind the remote chord-glitch investigation — a chord split across
two SendInput calls is the one sender-side place chord atomicity breaks.
"""

from sky_music.platform.win32 import inputs


def test_full_send_records_no_partial(monkeypatch):
    inputs.reset_send_diagnostics()
    monkeypatch.setattr(inputs.user32, "SendInput", lambda count, array, size: count)
    monkeypatch.setattr(inputs, "is_sky_active", lambda: True)

    inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=False)

    diag = inputs.get_send_diagnostics()
    assert diag == {
        "partial_send_events": 0,
        "chord_split_events": 0,
        "keys_deferred": 0,
        "zero_progress_retries": 0,
        "send_while_unfocused": 0,
        "impossible_same_key_repeats": 0,
    }


def test_partial_send_is_counted_as_chord_split(monkeypatch):
    inputs.reset_send_diagnostics()
    calls = {"n": 0}

    def fake_send_input(count, array, size):
        calls["n"] += 1
        # First call injects all-but-one (mid-chord partial); the deferred remainder then succeeds.
        return count - 1 if calls["n"] == 1 else count

    monkeypatch.setattr(inputs.user32, "SendInput", fake_send_input)
    monkeypatch.setattr(inputs, "is_sky_active", lambda: True)

    inputs.send_scan_code_batch_trusted((0x15, 0x16, 0x17), key_up=False)

    diag = inputs.get_send_diagnostics()
    assert diag["partial_send_events"] == 1
    assert diag["chord_split_events"] == 1
    assert diag["keys_deferred"] == 1


def test_single_key_partial_is_not_a_chord_split(monkeypatch):
    inputs.reset_send_diagnostics()
    calls = {"n": 0}

    def fake_send_input(count, array, size):
        calls["n"] += 1
        # First attempt injects nothing (blocked), retry succeeds — a stall, not a chord split.
        return 0 if calls["n"] == 1 else count

    monkeypatch.setattr(inputs.user32, "SendInput", fake_send_input)
    monkeypatch.setattr(inputs, "is_sky_active", lambda: True)

    inputs.send_scan_code_batch_trusted((0x15,), key_up=False)

    diag = inputs.get_send_diagnostics()
    # A blocked-then-succeed single key is a partial event but never a chord split.
    assert diag["chord_split_events"] == 0
    assert diag["partial_send_events"] == 1
    assert diag["keys_deferred"] == 1

def test_send_while_unfocused_counted_when_inactive():
    inputs.reset_send_diagnostics()
    
    # Counter is now explicitly bumped by the orchestration loop when focus is lost
    inputs.note_send_while_unfocused()

    diag = inputs.get_send_diagnostics()
    assert diag["send_while_unfocused"] == 1

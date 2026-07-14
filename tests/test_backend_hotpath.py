"""Phase B: zero-alloc decide paths + single-key bookkeeping on _TrackedKeyState.

Behaviour must match the pre-Phase-B semantics (duplicate-down, idempotent up, partial
send tracking). Allocation discipline is asserted via identity reuse of input tuples on
uniform batches — the free-threaded hot path must not build intermediate lists/tuples
when every key shares the same fate.
"""

from __future__ import annotations

from sky_music.infrastructure.backend import DryRunBackend, InputSendResult


def test_decide_down_reuses_tuple_when_all_free():
    backend = DryRunBackend()
    chord = (0x15, 0x16, 0x17)
    to_send, duplicates = backend._decide_down(chord)
    assert to_send is chord
    assert duplicates == ()


def test_decide_down_reuses_tuple_when_all_held():
    backend = DryRunBackend()
    chord = (0x15, 0x16, 0x17)
    backend.key_down(chord)
    to_send, duplicates = backend._decide_down(chord)
    assert to_send == ()
    assert duplicates is chord


def test_decide_down_single_key_no_split_lists():
    backend = DryRunBackend()
    one = (0x15,)
    to_send, duplicates = backend._decide_down(one)
    assert to_send is one
    assert duplicates == ()
    backend.key_down(one)
    to_send2, duplicates2 = backend._decide_down(one)
    assert to_send2 == ()
    assert duplicates2 is one


def test_decide_down_mixed_splits():
    backend = DryRunBackend()
    backend.key_down((0x15,))
    to_send, duplicates = backend._decide_down((0x15, 0x16, 0x17))
    assert to_send == (0x16, 0x17)
    assert duplicates == (0x15,)


def test_decide_up_reuses_tuple_when_all_held():
    backend = DryRunBackend()
    chord = (0x15, 0x16, 0x17)
    backend.key_down(chord)
    to_release, already = backend._decide_up(chord)
    assert to_release is chord
    assert already == ()


def test_decide_up_reuses_tuple_when_none_held():
    backend = DryRunBackend()
    chord = (0x15, 0x16, 0x17)
    to_release, already = backend._decide_up(chord)
    assert to_release == ()
    assert already is chord


def test_decide_up_mixed_splits():
    backend = DryRunBackend()
    backend.key_down((0x15, 0x16))
    to_release, already = backend._decide_up((0x15, 0x16, 0x17))
    assert to_release == (0x15, 0x16)
    assert already == (0x17,)


def test_single_key_roundtrip_state():
    backend = DryRunBackend()
    r_down = backend.key_down((0x15,))
    assert r_down.sent == (0x15,)
    assert backend.active_keys == {0x15}
    assert not backend.possibly_active_keys
    r_up = backend.key_up((0x15,))
    assert r_up.sent == (0x15,)
    assert not backend.active_keys
    assert not backend.possibly_active_keys


def test_chord_roundtrip_state():
    backend = DryRunBackend()
    chord = (0x15, 0x16, 0x17)
    r_down = backend.key_down(chord)
    assert r_down.sent == chord
    assert backend.active_keys == {0x15, 0x16, 0x17}
    r_up = backend.key_up(chord)
    assert r_up.sent == chord
    assert not backend.active_keys


def test_duplicate_down_partial_skip_unchanged():
    backend = DryRunBackend()
    backend.key_down((0x15,))
    r = backend.key_down((0x15, 0x16))
    assert r.sent == (0x16,)
    assert r.skipped_duplicates == (0x15,)
    assert backend.active_keys == {0x15, 0x16}


def test_key_up_idempotent_single():
    backend = DryRunBackend()
    r = backend.key_up((0x15,))
    assert r.success is True
    assert r.sent == ()
    assert r.skipped_duplicates == (0x15,)


def test_empty_batches():
    backend = DryRunBackend()
    assert backend.key_down(()) == InputSendResult(sent=(), skipped_duplicates=(), success=True)
    assert backend.key_up(()) == InputSendResult(sent=(), skipped_duplicates=(), success=True)

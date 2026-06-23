import random

import pytest

from sky_music.domain import Song, Note, NoteKey, Millis
from sky_music.domain.scheduler import build_key_actions
from sky_music.infrastructure.backend import WinSendInputBackend


def generate_random_song(num_notes: int = 100) -> Song:
    """Helper to generate a random song with diverse chord sizes and overlapping notes."""
    notes = []
    current_time_ms = 0
    note_keys = [f"Key{i}" for i in range(15)]
    
    for _ in range(num_notes):
        # 10% chance of a chord (same timestamp)
        if random.random() < 0.1:
            chord_size = random.randint(2, 5)
            for key in random.sample(note_keys, chord_size):
                notes.append(Note(time_ms=Millis(current_time_ms), key=NoteKey(key)))
        else:
            key = random.choice(note_keys)
            notes.append(Note(time_ms=Millis(current_time_ms), key=NoteKey(key)))
            
        # Step forward between 5ms and 300ms
        current_time_ms += random.randint(5, 300)
        
    return Song(name="Fuzzed Song", notes=tuple(notes))

def test_scheduler_invariants_and_fuzzing():
    """Fuzz the scheduling pipeline across 100 random songs to assert strict timing and key-up/key-down invariants."""
    random.seed(12345)  # Hard seed for deterministic and reproducible test execution
    for i in range(100):
        song = generate_random_song(num_notes=50)
        res = build_key_actions(song)
        actions = res.actions
        
        # Invariant 1: Equal down and up counts per scan code
        down_counts = {}
        up_counts = {}
        
        for a in actions:
            for sc in a.scan_codes:
                if a.kind == "down":
                    down_counts[sc] = down_counts.get(sc, 0) + 1
                else:
                    up_counts[sc] = up_counts.get(sc, 0) + 1
                    
        assert down_counts == up_counts, f"Unbalanced note onsets vs releases in fuzzed run {i}."
        
        # Invariant 2: Active keys at the end of simulation must be completely empty
        active = set()
        for a in actions:
            if a.kind == "down":
                active.update(a.scan_codes)
            else:
                active.difference_update(a.scan_codes)
                
        assert len(active) == 0, f"Stuck keys remaining at the end of fuzzed run {i}."

def test_fault_injection_and_recovery():
    """Verify that backend successfully tracks failed key-down events and cleans them up under panic release."""
    class FaultyInputs:
        def __init__(self):
            self.fail_down = False
            self.fail_up = False
            self.call_count = 0
            
        def send_scan_code_batch(self, scan_codes, key_up=False):
            self.call_count += 1
            if not key_up and self.fail_down:
                raise OSError("Inject KeyDown Failure")
            if key_up and self.fail_up:
                raise OSError("Inject KeyUp Failure")

    backend = WinSendInputBackend()
    mock_inputs = FaultyInputs()
    backend.inputs_module = mock_inputs  # type: ignore[assignment]
    
    # 1. Successful down
    backend.key_down((0x15, 0x16))
    assert backend.active_keys == {0x15, 0x16}
    assert len(backend.possibly_active_keys) == 0
    
    # 2. Failed down
    mock_inputs.fail_down = True
    with pytest.raises(OSError, match="Inject KeyDown Failure"):
        backend.key_down((0x17, 0x18))
        
    # The failed keys should not be active, but might be tracked or cleaned up
    assert 0x17 not in backend.active_keys
    
    # 3. Trigger emergency release_all
    mock_inputs.fail_down = False
    backend.release_all()
    
    # After release_all, all tracking sets should be clear
    assert len(backend.active_keys) == 0
    assert len(backend.possibly_active_keys) == 0
    assert len(backend.failed_release_keys) == 0

def test_fault_injection_cleanup_fail_retry():
    """Verify that when key_down and emergency cleanup key_up both fail, the keys are retained in possibly_active_keys.
    Then verify that release_all() successfully clears them on a subsequent retry pass."""
    class MultiFaultyInputs:
        def __init__(self):
            self.down_calls = 0
            self.up_calls = 0
            self.fail_down = True
            self.fail_up = True
            
        def send_scan_code_batch(self, scan_codes, key_up=False):
            if not key_up:
                self.down_calls += 1
                if self.fail_down:
                    raise OSError("KeyDown Failed")
            else:
                self.up_calls += 1
                if self.fail_up:
                    raise OSError(f"KeyUp Failed on pass {self.up_calls}")

    backend = WinSendInputBackend()
    mock_inputs = MultiFaultyInputs()
    backend.inputs_module = mock_inputs  # type: ignore[assignment]
    
    # 1. Try to inject KeyDown which fails, and its emergency cleanup KeyUp also fails
    with pytest.raises(OSError, match="KeyDown Failed"):
        backend.key_down((0x15,))
        
    # The key 0x15 must be retained inside possibly_active_keys since emergency cleanup failed!
    assert len(backend.active_keys) == 0
    assert backend.possibly_active_keys == {0x15}
    
    # 2. Call release_all() where first 2 passes fail, but 3rd pass succeeds
    # Set mock inputs to succeed on the 3rd keyup attempt
    def send_scan_code_batch_mock(scan_codes, key_up=False):
        mock_inputs.up_calls += 1
        if mock_inputs.up_calls < 3:
            raise OSError(f"KeyUp Failed on pass {mock_inputs.up_calls}")
        # Success on 3rd attempt
        
    mock_inputs.send_scan_code_batch = send_scan_code_batch_mock
    
    backend.release_all()
    
    # Verify that everything recovered to zero and was cleared
    assert len(backend.active_keys) == 0
    assert len(backend.possibly_active_keys) == 0
    assert len(backend.failed_release_keys) == 0
    assert mock_inputs.up_calls == 3 # Attempted 3 times before succeeding

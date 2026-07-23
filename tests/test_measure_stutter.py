import csv
import math
import struct
import subprocess
import sys
import wave
from pathlib import Path

import pytest

# Add tests directory to python path if needed
sys.path.insert(0, str(Path(__file__).parent))

def create_synthetic_wav(path: Path, onset_times: list[float], sr: int = 48000, duration: float = 3.0) -> None:
    """Generates a synthetic WAV with percussive blips at specified onset_times."""
    n_samples = int(duration * sr)
    samples = [0] * n_samples
    
    for t in onset_times:
        start_idx = int(t * sr)
        # Generate a 100ms decaying sound envelope
        for i in range(int(0.100 * sr)):
            idx = start_idx + i
            if idx >= n_samples:
                break
            t_onset = i / sr
            # Fast exponential decay to make it highly percussive
            env = math.exp(-40.0 * t_onset)
            val = math.sin(2.0 * math.pi * 440.0 * t_onset) * env
            samples[idx] = int(val * 32767)
            
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)  # 16-bit
        w.setframerate(sr)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))


def create_synthetic_csv(path: Path, actual_times_sec: list[float]) -> None:
    """Generates a synthetic telemetry CSV with specified down actual_us times."""
    headers = [
        "song", "event_index", "dispatch_id", "kind", "scheduled_us", "actual_us",
        "dispatch_completed_us", "evidence_scope", "lateness_us", "send_duration_us",
        "scan_codes", "sent_scan_codes", "skipped_scan_codes", "generation_ids",
        "runtime_outcome", "deferred_by_us", "pre_send_spin_us", "idle_gap_us", "reason"
    ]
    with open(path, "w", newline="", encoding="utf-8") as f:
        writer = csv.writer(f)
        writer.writerow(headers)
        for idx, t in enumerate(actual_times_sec):
            writer.writerow([
                "test", idx, idx, "down", int(t * 1_000_000), int(t * 1_000_000),
                int(t * 1_000_000), "sendinput_side", 0, 0,
                "21", "21", "", "0", "sent", 0, 0, 0, "test"
            ])


def test_imports():
    """Verify that we can import all required functions/symbols from the refactored script."""
    from measure_stutter import (
        analyze_and_report,
        best_offset,
        detect_onsets,
        load_sent_downs,
        match,
    )
    assert analyze_and_report is not None
    assert detect_onsets is not None
    assert best_offset is not None
    assert match is not None
    assert load_sent_downs is not None


def test_golden_regression(tmp_path: Path):
    """Ensure refactored CLI produces byte-for-byte identical output to the original CLI."""
    wav_path = tmp_path / "test.wav"
    csv_path = tmp_path / "test.csv"
    
    # 3 notes at 0.5s, 1.0s, 1.5s
    notes = [0.5, 1.0, 1.5]
    create_synthetic_wav(wav_path, notes)
    create_synthetic_csv(csv_path, notes)
    
    orig_script = Path(__file__).parent / "measure_stutter_original.py"
    new_script = Path(__file__).parent / "measure_stutter.py"
    
    args = [sys.executable, str(orig_script), str(wav_path), str(csv_path), "--fps", "60"]
    res_orig = subprocess.run(args, capture_output=True, text=True, encoding="utf-8")
    
    args_new = [sys.executable, str(new_script), str(wav_path), str(csv_path), "--fps", "60"]
    res_new = subprocess.run(args_new, capture_output=True, text=True, encoding="utf-8")
    
    # Compare outputs
    assert res_orig.returncode == res_new.returncode
    assert res_orig.stdout == res_new.stdout
    assert res_orig.stderr == res_new.stderr


def test_round_trip(tmp_path: Path):
    """Assert alignment converges and matches all notes perfectly (0 missing, low std/spread)."""
    from measure_stutter import (
        best_offset,
        detect_onsets,
        load_sent_downs,
        read_wav_mono,
    )
    from measure_stutter import (
        match as greedy_match,
    )
    
    wav_path = tmp_path / "roundtrip.wav"
    csv_path = tmp_path / "roundtrip.csv"
    
    # Generate 5 well-separated notes
    notes = [0.2, 0.7, 1.2, 1.7, 2.2]
    # We shift the recording onsets by +150ms to simulate a constant recording offset
    shifted_onsets = [n + 0.150 for n in notes]
    
    create_synthetic_wav(wav_path, shifted_onsets, sr=48000, duration=4.0)
    create_synthetic_csv(csv_path, notes)
    
    samples, sr = read_wav_mono(str(wav_path))
    detected = detect_onsets(samples, sr, hop_ms=5.0, min_gap_ms=40.0, sensitivity=1.5)
    sent, _intended, _counts = load_sent_downs(str(csv_path))
    
    # We expect exactly 5 detected onsets
    assert len(detected) == 5
    
    # Match and align
    tol = 0.120
    offset = best_offset(sent, detected, tol)
    
    # Offset should be close to 150ms (0.150s)
    assert abs(offset - 0.150) < 0.02
    
    pairs, missing, extra = greedy_match(sent, detected, offset, tol)
    
    assert len(missing) == 0
    assert len(pairs) == len(notes)
    assert len(extra) == 0


def test_audio_loopback_import_and_probe(tmp_path: Path):
    """Verify audio_loopback module exists, can be imported, and has device probe."""
    try:
        import audio_loopback
    except ImportError:
        pytest.skip("audio_loopback dependencies (e.g. soundcard) not installed.")
        
    assert hasattr(audio_loopback, "capture_loopback_to_wav")
    assert hasattr(audio_loopback, "get_default_loopback_device_name")
    
    device_name = audio_loopback.get_default_loopback_device_name()
    if device_name is None:
        pytest.skip("No default audio output/loopback device is available on this system.")
        
    # Try capturing a short 0.2s duration to verify recording and writing works
    out_wav = tmp_path / "probe_capture.wav"
    try:
        audio_loopback.capture_loopback_to_wav(out_wav, max_seconds=0.2)
    except RuntimeError as e:
        if "no audio samples were recorded" in str(e):
            pytest.skip("Audio device exists but recording returned no samples (headless/VM environment).")
        raise
    assert out_wav.exists()
    
    # Verify read_wav_mono reads it successfully
    from measure_stutter import read_wav_mono
    mono, sr = read_wav_mono(str(out_wav))
    assert len(mono) > 0
    assert sr == 48000


def test_live_cli_execution(tmp_path: Path):
    """Verify that measure_stutter_live.py runs cleanly with command line options."""
    try:
        import audio_loopback
        device_name = audio_loopback.get_default_loopback_device_name()
        if device_name is None:
            pytest.skip("No default audio output/loopback device is available on this system.")
    except ImportError:
        pytest.skip("audio_loopback dependencies (e.g. soundcard) not installed.")
        
    csv_path = tmp_path / "live_test.csv"
    create_synthetic_csv(csv_path, [0.1])
    
    live_script = Path(__file__).parent / "measure_stutter_live.py"
    
    # Run with 0.2s duration and specified CSV
    args = [
        sys.executable,
        str(live_script),
        "--duration", "0.2",
        "--csv", str(csv_path),
        "--fps", "60"
    ]
    res = subprocess.run(args, capture_output=True, text=True, encoding="utf-8")
    
    # Assert successful execution
    assert "Traceback" not in res.stderr
    assert "ERROR" not in res.stderr
    assert "Wrote captured WAV to:" in res.stdout
    assert ("=== 1. MISSING NOTES" in res.stdout or "Not enough data to align" in res.stdout)

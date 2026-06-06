"""WASAPI loopback audio capture module.

Captures system audio playback via Windows WASAPI Loopback utilizing the soundcard library
and writes it as a 16-bit PCM WAV.

Install optional dependency via:
    uv add --dev soundcard
"""
from __future__ import annotations

import time
import threading
import wave
from pathlib import Path

try:
    import soundcard as sc
    import numpy as np
    HAS_SOUNDCARD = True
except ImportError:
    HAS_SOUNDCARD = False
    sc = None
    np = None


def get_default_loopback_device_name() -> str | None:
    """Return the name of the default loopback device, or None if unavailable/unsupported."""
    if not HAS_SOUNDCARD:
        return None
    try:
        speaker = sc.default_speaker()
        mic = sc.get_microphone(speaker.name, include_loopback=True)
        return mic.name
    except Exception:
        return None


def capture_loopback_to_wav(
    out_path: Path,
    *,
    stop_event: threading.Event | None = None,
    max_seconds: float | None = None,
    samplerate: int = 48_000,
) -> Path:
    """Capture system render stream via WASAPI loopback to a 16-bit PCM WAV.

    Stops when stop_event is set or after max_seconds, whichever is first.
    """
    if not HAS_SOUNDCARD:
        raise RuntimeError(
            "Optional dependency 'soundcard' is not installed.\n"
            "Please install it using:\n"
            "  uv add --dev soundcard"
        )

    try:
        speaker = sc.default_speaker()
        mic = sc.get_microphone(speaker.name, include_loopback=True)
    except Exception as e:
        raise RuntimeError(
            f"Failed to locate default audio loopback device: {e}\n"
            "Ensure you have a default audio output device enabled in Windows Sound settings."
        )

    channels = 1
    # Record in small chunks (e.g., 100ms blocks) to allow timely stop_event checks
    block_duration = 0.1
    block_frames = int(samplerate * block_duration)

    start_time = time.time()
    all_chunks = []

    try:
        with mic.recorder(samplerate=samplerate, channels=channels) as recorder:
            while True:
                # Check stop event
                if stop_event is not None and stop_event.is_set():
                    break

                # Check elapsed time
                elapsed = time.time() - start_time
                if max_seconds is not None and elapsed >= max_seconds:
                    break

                # Calculate remaining frames to read
                frames_to_read = block_frames
                if max_seconds is not None:
                    remaining_sec = max_seconds - elapsed
                    if remaining_sec < block_duration:
                        frames_to_read = max(1, int(samplerate * remaining_sec))

                # Record a chunk
                chunk = recorder.record(numframes=frames_to_read)
                all_chunks.append(chunk)

                # Stop if max_seconds is reached
                if max_seconds is not None and (elapsed + frames_to_read / samplerate) >= max_seconds:
                    break
    except Exception as e:
        raise RuntimeError(f"Error during audio loopback capture: {e}")

    if not all_chunks:
        raise RuntimeError("Capture completed but no audio samples were recorded.")

    # Concatenate all numpy array chunks
    data = np.concatenate(all_chunks, axis=0)

    # Convert float32 -> 16-bit PCM (clamp to [-1.0, 1.0], scale to signed int16)
    clipped = np.clip(data, -1.0, 1.0)
    int_data = (clipped * 32767.0).astype(np.int16)

    # Ensure parent directory exists
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    # Write WAV file using stdlib wave
    with wave.open(str(out_path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)  # 16-bit PCM
        w.setframerate(samplerate)
        w.writeframes(int_data.tobytes())

    return out_path

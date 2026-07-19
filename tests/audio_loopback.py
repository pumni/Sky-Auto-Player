"""WASAPI loopback audio capture module.

Captures system audio playback via Windows WASAPI Loopback utilizing the soundcard library
and writes it as a 16-bit PCM WAV.

Install optional dependency via:
    uv add --dev soundcard
"""
from __future__ import annotations

import threading
import time
import wave
from pathlib import Path

try:
    import numpy as np
    import soundcard as sc
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
        assert sc is not None
        speaker = sc.default_speaker()
        mic = sc.get_microphone(speaker.name, include_loopback=True)
        return mic.name
    except Exception:
        return None


def _keep_alive_loop(speaker, stop_event: threading.Event, samplerate: int) -> None:
    """Play silence to keep WASAPI loopback from blocking when the system is quiet."""
    if np is None:
        return
    try:
        with speaker.player(samplerate=samplerate, channels=1) as sp:
            zeros = np.zeros(max(1, int(samplerate * 0.1)), dtype=np.float32)
            while not stop_event.is_set():
                sp.play(zeros)
    except Exception:
        pass


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
        assert sc is not None
        speaker = sc.default_speaker()
        mic = sc.get_microphone(speaker.name, include_loopback=True)
    except Exception as e:
        raise RuntimeError(
            f"Failed to locate default audio loopback device: {e}\n"
            "Ensure you have a default audio output device enabled in Windows Sound settings."
        ) from e

    channels = 1
    # Record in small chunks (e.g., 100ms blocks) to allow timely stop_event checks
    block_duration = 0.1
    block_frames = int(samplerate * block_duration)

    start_time = time.time()
    all_chunks = []

    # Start a dummy playback thread to keep WASAPI loopback active during silence.
    # Otherwise loopback will pause and drop silent frames, corrupting timestamps.
    keep_alive_stop = threading.Event()
    keep_alive_thread = threading.Thread(
        target=_keep_alive_loop,
        args=(speaker, keep_alive_stop, samplerate),
        daemon=True,
    )
    keep_alive_thread.start()

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
        raise RuntimeError(f"Error during audio loopback capture: {e}") from e
    finally:
        keep_alive_stop.set()
        keep_alive_thread.join(timeout=0.2)

    if not all_chunks:
        raise RuntimeError("Capture completed but no audio samples were recorded.")

    # Concatenate all numpy array chunks
    assert np is not None
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

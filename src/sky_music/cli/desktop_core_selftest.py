"""Black-box self-test for the packaged Desktop Core executable.

The test intentionally speaks the same bounded NDJSON protocol as Tauri.  It
does not import application services from the child process, which makes it a
useful packaging check rather than another in-process unit test.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any

from sky_music.infrastructure.desktop_ipc.protocol import (
    encode_frame,
    iter_bounded_frames,
)

_TIMEOUT_S = 15.0
_MAX_STDERR_BYTES = 128 * 1024


class _ChildOutput:
    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        self.frames: queue.Queue[object] = queue.Queue()
        self.stderr = bytearray()
        self._stderr_lock = threading.Lock()
        self._threads = [
            threading.Thread(target=self._read_stdout, args=(process.stdout,), daemon=True),
            threading.Thread(target=self._read_stderr, args=(process.stderr,), daemon=True),
        ]
        for thread in self._threads:
            thread.start()

    def _read_stdout(self, stream: Any) -> None:
        try:
            for frame in iter_bounded_frames(stream):
                self.frames.put(frame)
        except BaseException as exc:  # delivered to the parent test below
            self.frames.put(exc)
        finally:
            self.frames.put(None)

    def _read_stderr(self, stream: Any) -> None:
        try:
            while True:
                chunk = stream.read1(4096) if hasattr(stream, "read1") else stream.read(4096)
                if not chunk:
                    return
                with self._stderr_lock:
                    remaining = _MAX_STDERR_BYTES - len(self.stderr)
                    if remaining > 0:
                        self.stderr.extend(chunk[:remaining])
        except BaseException:
            return


def _root_and_command() -> tuple[Path, list[str]]:
    executable = Path(sys.executable).resolve()
    override = os.environ.get("SKY_DESKTOP_SELFTEST_CHILD")
    if override:
        try:
            values = json.loads(override)
        except json.JSONDecodeError as exc:
            raise RuntimeError("SKY_DESKTOP_SELFTEST_CHILD is not valid JSON") from exc
        if (
            not isinstance(values, list)
            or not values
            or any(not isinstance(value, str) or not value for value in values)
        ):
            raise RuntimeError("SKY_DESKTOP_SELFTEST_CHILD must be a non-empty string list")
        return executable.parent, values
    if getattr(sys, "frozen", False):
        return executable.parent, [str(executable)]
    root = Path(__file__).resolve().parents[3]
    return root, [sys.executable, str(root / "src" / "core_main.py")]


def run_core_selftest(
    *,
    command: list[str] | None = None,
    root: Path | None = None,
    timeout_s: float = _TIMEOUT_S,
) -> int:
    """Run a bounded ready/bootstrap/request/shutdown round-trip.

    Diagnostics are written to stderr so the method remains safe to use from
    an automated package gate and never contaminates Desktop Core stdout.
    """
    default_root, default_command = _root_and_command()
    install_root = root if root is not None else default_root
    child_command = list(command) if command is not None else default_command
    if not install_root.is_dir():
        print(
            f"desktop Core selftest: install root missing: {install_root}",
            file=sys.stderr,
        )
        return 2
    child_command += [
        "--desktop-worker",
        "--parent-pid",
        str(os.getpid()),
        "--install-root",
        str(install_root),
    ]
    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    try:
        process = subprocess.Popen(
            child_command,
            cwd=str(install_root),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            creationflags=creationflags,
        )
    except OSError as exc:
        print(f"desktop Core selftest: launch failed: {exc}", file=sys.stderr)
        return 2

    output = _ChildOutput(process)
    next_id = 1

    def receive(timeout: float = timeout_s) -> dict[str, object] | None:
        try:
            item = output.frames.get(timeout=timeout)
        except queue.Empty:
            raise RuntimeError("timed out waiting for Core protocol frame") from None
        if item is None:
            return None
        if isinstance(item, BaseException):
            raise RuntimeError(f"Core stdout protocol failure: {item}") from item
        if not isinstance(item, bytes):
            raise RuntimeError("Core stdout reader returned an invalid frame")
        try:
            import json

            decoded = json.loads(item.decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as exc:
            raise RuntimeError("Core emitted malformed JSON") from exc
        if not isinstance(decoded, dict):
            raise RuntimeError("Core emitted a non-object protocol frame")
        return decoded

    def request(method: str, params: dict[str, object]) -> dict[str, object]:
        nonlocal next_id
        request_id = next_id
        next_id += 1
        frame = {
            "v": 1,
            "id": request_id,
            "type": "request",
            "method": method,
            "params": params,
        }
        assert process.stdin is not None
        process.stdin.write(encode_frame(frame))
        process.stdin.flush()
        while True:
            message = receive()
            if message is None:
                raise RuntimeError(f"Core exited while awaiting {method}")
            if message.get("type") == "event":
                continue
            if message.get("id") != request_id:
                raise RuntimeError(f"unexpected Core response ID for {method}")
            if message.get("ok") is not True:
                raise RuntimeError(f"Core rejected {method}: {message.get('error')!r}")
            result = message.get("result")
            if not isinstance(result, dict):
                raise RuntimeError(f"Core returned invalid {method} result")
            return result

    try:
        ready = receive()
        if ready is None or ready.get("type") != "event" or ready.get("name") != "core.ready":
            raise RuntimeError("Core did not emit core.ready")
        bootstrap = request("app.bootstrap", {})
        if not isinstance(bootstrap.get("native_build"), dict):
            raise RuntimeError("bootstrap omitted native build information")
        request("settings.get", {})
        request("app.shutdown", {})
        # Closing the parent end makes the inherited stdin EOF explicit.  The
        # Core has already completed the bounded shutdown response; this also
        # lets its reader thread leave a blocking pipe read on Windows.
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=timeout_s)
        except subprocess.TimeoutExpired as exc:
            process.kill()
            process.wait(timeout=2)
            raise RuntimeError("Core did not exit after app.shutdown") from exc
        if process.returncode != 0:
            raise RuntimeError(f"Core exited with status {process.returncode}")
        print("Packaged Desktop Core selftest: PASS")
        return 0
    except (OSError, RuntimeError) as exc:
        print(f"desktop Core selftest: FAIL: {exc}", file=sys.stderr)
        if process.poll() is None:
            process.kill()
            process.wait(timeout=2)
        return 1


def run_packaged_core_selftest() -> int:
    raw_timeout = os.environ.get("SKY_DESKTOP_SELFTEST_TIMEOUT_SECONDS")
    timeout = _TIMEOUT_S
    if raw_timeout is not None:
        try:
            timeout = float(raw_timeout)
        except ValueError:
            print("desktop Core selftest: invalid timeout override", file=sys.stderr)
            return 2
        if not 0.1 <= timeout <= 60:
            print("desktop Core selftest: timeout override is out of bounds", file=sys.stderr)
            return 2
    return run_core_selftest(timeout_s=timeout)


__all__ = ["run_core_selftest", "run_packaged_core_selftest"]

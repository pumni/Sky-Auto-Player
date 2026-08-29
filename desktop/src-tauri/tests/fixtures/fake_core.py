"""Deterministic child process used by the Rust CoreSupervisor tests.

The fixture deliberately speaks the same bounded NDJSON protocol as the real
desktop Core, but never touches the filesystem, native input, or game process.
"""
from __future__ import annotations

import json
import sys
import time
from typing import Any


def emit(message: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def raw(data: bytes) -> None:
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()


def ready() -> None:
    emit(
        {
            "v": 1,
            "type": "event",
            "name": "core.ready",
            "payload": {
                "app_version": "fake-core",
                "protocol_version": 1,
                "native_build": {
                    "native_build_commit": "a" * 40,
                    "native_version": "3.5.0",
                    "schema_version": 10,
                    "native_abi": "cp314t-win_amd64",
                    "rustc_version": "1.98.0",
                    "win32_backend": True,
                },
            },
        }
    )


def fatal(message: str = "fake Core failure") -> None:
    emit(
        {
            "v": 1,
            "type": "event",
            "name": "core.fatal",
            "payload": {"code": "fake_failure", "message": message},
        }
    )


def playback_state(*, physical: bool) -> None:
    emit(
        {
            "v": 1,
            "type": "event",
            "name": "playback.state_changed",
            "payload": {
                "session_id": "b" * 32,
                "song_id": "c" * 32,
                "state": "playing",
                "physical": physical,
                "message": None,
                "outcome": None,
            },
        }
    )


def response(request_id: int, result: Any = None) -> None:
    emit(
        {
            "v": 1,
            "id": request_id,
            "type": "response",
            "ok": True,
            "result": {} if result is None else result,
        }
    )


def command_result(item: dict[str, Any]) -> Any:
    method = item.get("method")
    if method == "diagnostics.set_enabled":
        return {"enabled": item.get("params", {}).get("enabled", False)}
    if method == "calibration.start":
        return {"operation_id": "d" * 32, "state": "running"}
    if method == "calibration.cancel":
        return {
            "operation_id": item.get("params", {}).get("operation_id", "d" * 32),
            "state": "cancelled",
            "accepted": True,
        }
    if method == "catalog.search":
        return {
            "items": [],
            "offset": int(item.get("params", {}).get("offset", 0)),
            "limit": int(item.get("params", {}).get("limit", 200)),
            "total": 0,
            "generation": 1,
        }
    if method == "playback.prepare":
        params = item.get("params", {})
        song_id = params.get("song_id", "c" * 32)
        return {
            "prepared_id": "a" * 32,
            "song": {
                "song_id": song_id,
                "title": "Fake Song",
                "duration_us": 100,
                "note_count": 1,
                "format_label": "JSON",
                "risk": {
                    "level": "low",
                    "headline": "Low timing risk",
                    "reasons": [],
                    "recommendations": [],
                },
                "recommendation": None,
            },
            "config": params.get("config", {
                "hold_frames": 1.0,
                "tempo_scale": 1.0,
                "fps": 60,
                "dry_run": True,
            }),
            "admission": "ready",
            "risk": {
                "level": "low",
                "headline": "Low timing risk",
                "reasons": [],
                "recommendations": [],
            },
            "decisions": [],
            "plan_fingerprint": "fake-plan",
            "variants": [
                {
                    "decision": "proceed",
                    "config": params.get("config", {
                        "hold_frames": 1.0,
                        "tempo_scale": 1.0,
                        "fps": 60,
                        "dry_run": True,
                    }),
                    "plan_fingerprint": "fake-plan",
                }
            ],
            "error_code": None,
            "error_message": None,
        }
    if method == "playback.start":
        return {
            "session_id": "b" * 32,
            "prepared_id": item.get("params", {}).get("prepared_id", "a" * 32),
            "song_id": "c" * 32,
            "state": "starting",
            "config": {
                "hold_frames": 1.0,
                "tempo_scale": 1.0,
                "fps": 60,
                "dry_run": True,
            },
            "plan_fingerprint": "fake-plan",
        }
    if method in {"playback.stop", "playback.pause", "playback.resume", "playback.skip"}:
        return {
            "accepted": True,
            "session_id": "b" * 32,
            "state": "playing",
            "pending_command": None,
            "reason": None,
        }
    return {}


def request() -> dict[str, Any] | None:
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    return json.loads(line)


def serve(mode: str) -> None:
    if mode == "startup_timeout":
        time.sleep(30)
        return
    if mode == "eof_before_ready":
        return
    if mode == "malformed":
        raw(b"{not valid json}\n")
        time.sleep(30)
        return
    if mode == "duplicate_output":
        raw(b'{"v":1,"type":"event","name":"core.ready","name":"again","payload":{}}\n')
        time.sleep(30)
        return
    if mode == "oversized_output":
        raw(b"x" * (1024 * 1024 + 1) + b"\n")
        time.sleep(30)
        return

    if mode == "fatal_before_ready":
        fatal("fatal before ready")
        time.sleep(30)
        return

    ready()
    if mode == "duplicate_ready":
        ready()
        time.sleep(30)
        return
    if mode == "fatal_after_ready":
        fatal("fatal after ready")
        time.sleep(30)
        return
    if mode == "physical_active_exit":
        playback_state(physical=True)
        return
    if mode == "physical_active_fatal":
        playback_state(physical=True)
        fatal("fatal during physical playback")
        time.sleep(30)
        return
    if mode == "dry_run_active_exit":
        playback_state(physical=False)
        return
    if mode == "eof_after_ready":
        return
    if mode == "stderr_flood":
        sys.stderr.buffer.write(b"diagnostic\n" * 250_000)
        sys.stderr.buffer.flush()
    if mode == "request_timeout":
        item = request()
        if item is not None:
            time.sleep(0.25)
            response(int(item["id"]), {"late": True})
        while True:
            item = request()
            if item is None:
                return
            if item.get("method") == "app.shutdown":
                response(int(item["id"]))
                return
            response(int(item["id"]), {"method": item.get("method")})
    if mode == "unknown_id":
        item = request()
        if item is not None:
            response(int(item["id"]) + 1)
        time.sleep(30)
        return
    if mode == "child_pending":
        request()
        return
    if mode == "force_shutdown":
        while True:
            item = request()
            if item is None:
                return
            if item.get("method") == "app.shutdown":
                time.sleep(30)
            else:
                response(int(item["id"]))

    while True:
        item = request()
        if item is None:
            return
        if item.get("method") == "app.shutdown":
            response(int(item["id"]))
            return
        if mode == "tauri_commands":
            response(int(item["id"]), command_result(item))
            continue
        response(
            int(item["id"]),
            {"method": item.get("method"), "params": item.get("params")},
        )


if __name__ == "__main__":
    serve(sys.argv[1] if len(sys.argv) > 1 else "normal")

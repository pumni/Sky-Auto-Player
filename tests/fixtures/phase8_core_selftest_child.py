"""Small protocol child used to exercise the packaged-Core selftest harness."""

from __future__ import annotations

import json
import sys
import time


def emit(value: object) -> None:
    sys.stdout.buffer.write(json.dumps(value, separators=(",", ":")).encode() + b"\n")
    sys.stdout.buffer.flush()


def ready() -> None:
    emit(
        {
            "v": 1,
            "type": "event",
            "name": "core.ready",
            "payload": {
                "app_version": "selftest-fixture",
                "protocol_version": 1,
                "native_build": {
                    "native_build_commit": "a" * 40,
                    "native_version": "test",
                    "schema_version": 1,
                    "native_abi": "test",
                    "rustc_version": "test",
                    "win32_backend": False,
                },
            },
        }
    )


def main() -> int:
    scenario = sys.argv[1]
    if scenario == "exit_before_ready":
        return 0
    if scenario == "startup_fatal":
        emit(
            {
                "v": 1,
                "type": "event",
                "name": "core.fatal",
                "payload": {"code": "fixture", "message": "startup failed"},
            }
        )
        return 0
    if scenario == "malformed":
        sys.stdout.buffer.write(b"{not-json}\n")
        sys.stdout.buffer.flush()
        return 0
    if scenario == "non_utf8":
        sys.stdout.buffer.write(b"\xff\xfe\n")
        sys.stdout.buffer.flush()
        return 0
    ready()
    for line in sys.stdin.buffer:
        request = json.loads(line)
        request_id = request.get("id")
        method = request.get("method")
        if scenario == "wrong_response_id":
            request_id = int(request_id) + 1
        if scenario == "request_timeout":
            time.sleep(5)
            continue
        if method == "app.bootstrap":
            emit(
                {
                    "v": 1,
                    "id": request_id,
                    "type": "response",
                    "ok": True,
                    "result": {"native_build": {}},
                }
            )
        elif method == "settings.get":
            emit(
                {
                    "v": 1,
                    "id": request_id,
                    "type": "response",
                    "ok": True,
                    "result": {},
                }
            )
        elif method == "app.shutdown":
            emit(
                {
                    "v": 1,
                    "id": request_id,
                    "type": "response",
                    "ok": True,
                    "result": {"shutdown": True},
                }
            )
            if scenario == "shutdown_hang":
                time.sleep(5)
            return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

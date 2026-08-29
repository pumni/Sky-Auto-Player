"""Strict, bounded NDJSON primitives for the desktop Core boundary."""

from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from typing import Any

DESKTOP_PROTOCOL_VERSION = 1
MAX_INBOUND_FRAME_BYTES = 64 * 1024
MAX_OUTBOUND_FRAME_BYTES = 1 * 1024 * 1024
# Descriptive aliases used by callers that name limits by protocol direction.
MAX_REQUEST_FRAME_BYTES = MAX_INBOUND_FRAME_BYTES
MAX_RESPONSE_FRAME_BYTES = MAX_OUTBOUND_FRAME_BYTES
MAX_ERROR_MESSAGE_BYTES = 4 * 1024
MAX_METHOD_BYTES = 128
MAX_REQUEST_ID = 2**53 - 1
_READ_CHUNK_BYTES = 4096
_REQUEST_KEYS = frozenset({"v", "id", "type", "method", "params"})


class ProtocolError(ValueError):
    """A malformed or unsupported protocol frame."""

    def __init__(self, code: str, message: str, *, request_id: int | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.request_id = request_id


def _reject_constant(value: str) -> object:
    raise ProtocolError("invalid_json", f"non-finite JSON constant is not allowed: {value}")


def _reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ProtocolError("invalid_json", f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _as_bytes(frame: bytes | bytearray | memoryview | str) -> bytes:
    if isinstance(frame, str):
        return frame.encode("utf-8")
    return bytes(frame)


def _request_id(value: Mapping[str, object]) -> int | None:
    candidate = value.get("id")
    if type(candidate) is int and 0 <= candidate <= MAX_REQUEST_ID:
        return candidate
    return None


def validate_request_object(value: object) -> dict[str, object]:
    """Validate an already-decoded request object without coercion."""
    if not isinstance(value, dict):
        raise ProtocolError("invalid_request", "request must be a JSON object")
    request_id = _request_id(value)
    if any(not isinstance(key, str) for key in value):
        raise ProtocolError("invalid_request", "request keys must be strings")
    keys = frozenset(value)
    if keys != _REQUEST_KEYS:
        missing = sorted(_REQUEST_KEYS - keys)
        unknown = sorted(keys - _REQUEST_KEYS)
        details = []
        if missing:
            details.append(f"missing fields: {', '.join(missing)}")
        if unknown:
            details.append(f"unknown fields: {', '.join(unknown)}")
        raise ProtocolError("invalid_request", "; ".join(details), request_id=request_id)
    version = value["v"]
    if type(version) is not int or version != DESKTOP_PROTOCOL_VERSION:
        raise ProtocolError(
            "unsupported_version",
            f"unsupported protocol version: {version!r}",
            request_id=request_id,
        )
    if type(value["id"]) is not int or not 0 <= value["id"] <= MAX_REQUEST_ID:
        raise ProtocolError("invalid_id", "request id must be an integer in the safe JSON range")
    if value["type"] != "request":
        raise ProtocolError("invalid_type", "message type must be request", request_id=request_id)
    method = value["method"]
    if not isinstance(method, str) or not method or len(method.encode("utf-8")) > MAX_METHOD_BYTES:
        raise ProtocolError("invalid_method", "method must be a bounded non-empty string", request_id=request_id)
    if any(char not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.-" for char in method):
        raise ProtocolError("invalid_method", "method contains unsupported characters", request_id=request_id)
    if not isinstance(value["params"], dict):
        raise ProtocolError("invalid_params", "params must be an object", request_id=request_id)
    return value


def parse_request_frame(frame: bytes | bytearray | memoryview | str) -> dict[str, object]:
    """Decode and strictly validate one bounded NDJSON request frame."""
    raw = _as_bytes(frame)
    if raw.endswith(b"\n"):
        raw = raw[:-1]
    if raw.endswith(b"\r"):
        raw = raw[:-1]
    if len(raw) > MAX_INBOUND_FRAME_BYTES:
        raise ProtocolError("frame_too_large", "request frame exceeds 64 KiB")
    try:
        text = raw.decode("utf-8")
        decoded = json.loads(
            text,
            parse_constant=_reject_constant,
            object_pairs_hook=_reject_duplicate_keys,
        )
    except ProtocolError:
        raise
    except (RecursionError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProtocolError("invalid_json", "request is not valid UTF-8 JSON") from exc
    return validate_request_object(decoded)


def _bounded_text(value: object) -> str:
    text = str(value).replace("\x00", "")
    encoded = text.encode("utf-8", errors="replace")
    if len(encoded) <= MAX_ERROR_MESSAGE_BYTES:
        return text
    return encoded[: MAX_ERROR_MESSAGE_BYTES - 3].decode("utf-8", errors="ignore") + "..."


def bounded_text(value: object) -> str:
    """Return text safe for a bounded user-facing protocol field."""
    return _bounded_text(value)


def encode_frame(message: Mapping[str, object]) -> bytes:
    """Serialize one protocol message and enforce the outbound frame limit."""
    try:
        encoded = json.dumps(
            dict(message),
            ensure_ascii=False,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8") + b"\n"
    except (TypeError, ValueError) as exc:
        raise ProtocolError("invalid_response", "response contains unsupported JSON values") from exc
    if len(encoded) > MAX_OUTBOUND_FRAME_BYTES:
        raise ProtocolError("frame_too_large", "response frame exceeds 1 MiB")
    return encoded


def response_ok(request_id: int, result: Mapping[str, object]) -> dict[str, object]:
    return {"v": DESKTOP_PROTOCOL_VERSION, "id": request_id, "type": "response", "ok": True, "result": dict(result)}


def response_error(request_id: int, code: str, message: object) -> dict[str, object]:
    return {
        "v": DESKTOP_PROTOCOL_VERSION,
        "id": request_id,
        "type": "response",
        "ok": False,
        "error": {"code": _bounded_text(code), "message": _bounded_text(message)},
    }


def event(name: str, payload: Mapping[str, object]) -> dict[str, object]:
    return {
        "v": DESKTOP_PROTOCOL_VERSION,
        "type": "event",
        "name": name,
        "payload": dict(payload),
    }


def iter_bounded_frames(stream: Any) -> Iterable[bytes]:
    """Yield newline-delimited frames without calling unbounded ``readline``."""
    pending = bytearray()
    while True:
        chunk = stream.read(_READ_CHUNK_BYTES)
        if chunk in (b"", ""):
            break
        if isinstance(chunk, str):
            chunk = chunk.encode("utf-8")
        if not isinstance(chunk, bytes):
            raise ProtocolError("stream_error", "stdin returned a non-byte chunk")
        pending.extend(chunk)
        while True:
            newline = pending.find(b"\n")
            if newline < 0:
                break
            line = bytes(pending[:newline])
            del pending[: newline + 1]
            if len(line) > MAX_INBOUND_FRAME_BYTES:
                raise ProtocolError("frame_too_large", "request frame exceeds 64 KiB")
            yield line
        if len(pending) > MAX_INBOUND_FRAME_BYTES:
            raise ProtocolError("frame_too_large", "request frame exceeds 64 KiB")
    if pending:
        if len(pending) > MAX_INBOUND_FRAME_BYTES:
            raise ProtocolError("frame_too_large", "request frame exceeds 64 KiB")
        yield bytes(pending)


def write_frame(stream: Any, message: Mapping[str, object]) -> None:
    encoded = encode_frame(message)
    try:
        stream.write(encoded)
    except TypeError:
        stream.write(encoded.decode("utf-8"))
    flush = getattr(stream, "flush", None)
    if callable(flush):
        flush()


__all__ = [
    "DESKTOP_PROTOCOL_VERSION",
    "MAX_ERROR_MESSAGE_BYTES",
    "MAX_INBOUND_FRAME_BYTES",
    "MAX_OUTBOUND_FRAME_BYTES",
    "MAX_REQUEST_FRAME_BYTES",
    "MAX_REQUEST_ID",
    "MAX_RESPONSE_FRAME_BYTES",
    "ProtocolError",
    "bounded_text",
    "encode_frame",
    "event",
    "iter_bounded_frames",
    "parse_request_frame",
    "response_error",
    "response_ok",
    "validate_request_object",
    "write_frame",
]

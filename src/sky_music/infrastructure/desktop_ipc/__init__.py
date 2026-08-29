"""Versioned bounded stdin/stdout protocol for the Python desktop Core."""

from sky_music.infrastructure.desktop_ipc.protocol import (
    DESKTOP_PROTOCOL_VERSION,
    MAX_ERROR_MESSAGE_BYTES,
    MAX_INBOUND_FRAME_BYTES,
    MAX_OUTBOUND_FRAME_BYTES,
    MAX_REQUEST_FRAME_BYTES,
    MAX_RESPONSE_FRAME_BYTES,
)

__all__ = [
    "DESKTOP_PROTOCOL_VERSION",
    "MAX_ERROR_MESSAGE_BYTES",
    "MAX_INBOUND_FRAME_BYTES",
    "MAX_OUTBOUND_FRAME_BYTES",
    "MAX_REQUEST_FRAME_BYTES",
    "MAX_RESPONSE_FRAME_BYTES",
]

"""Fail-safe persistent envelope for the native adaptive-lead estimator."""

from __future__ import annotations

import json
import logging
import os
from collections.abc import Mapping
from pathlib import Path
from typing import Any

_LOGGER = logging.getLogger(__name__)
ESTIMATOR_CACHE_SCHEMA_VERSION = 1
DEFAULT_ESTIMATOR_CACHE = Path("logs") / "native_estimator_state.json"


def load_estimator_state(
    *,
    game_fps: int,
    native_build_commit: str,
    native_abi: str,
    path: Path = DEFAULT_ESTIMATOR_CACHE,
) -> str | None:
    try:
        envelope = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(envelope, Mapping):
            raise ValueError("cache envelope is not an object")
        if envelope.get("schema_version") != ESTIMATOR_CACHE_SCHEMA_VERSION:
            raise ValueError("cache schema mismatch")
        if envelope.get("native_build_commit") != native_build_commit:
            raise ValueError("native build mismatch")
        if envelope.get("native_abi") != native_abi:
            raise ValueError("native ABI mismatch")
        if envelope.get("dispatch_schema_version") not in (2, 3):
            raise ValueError("dispatch schema mismatch")
        if envelope.get("game_fps") != game_fps:
            raise ValueError("game FPS mismatch")
        state = envelope.get("estimator_state_json")
        if not isinstance(state, str) or not isinstance(json.loads(state), Mapping):
            raise ValueError("estimator state is not a JSON object")
        return state
    except (OSError, TypeError, ValueError, json.JSONDecodeError) as exc:
        if path.exists():
            _LOGGER.debug("ignoring adaptive estimator cache: %s", exc)
        return None


def save_estimator_state(
    state_json: str,
    *,
    game_fps: int,
    native_build_commit: str,
    native_abi: str,
    path: Path = DEFAULT_ESTIMATOR_CACHE,
) -> None:
    try:
        if not isinstance(json.loads(state_json), Mapping):
            raise ValueError("estimator state is not a JSON object")
        envelope: dict[str, Any] = {
            "schema_version": ESTIMATOR_CACHE_SCHEMA_VERSION,
            "native_build_commit": native_build_commit,
            "native_abi": native_abi,
            "dispatch_schema_version": 3,
            "game_fps": game_fps,
            "estimator_state_json": state_json,
        }
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
        with temporary.open("w", encoding="utf-8", newline="\n") as handle:
            json.dump(envelope, handle, separators=(",", ":"), sort_keys=True)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except (OSError, TypeError, ValueError, json.JSONDecodeError) as exc:
        _LOGGER.debug("could not persist adaptive estimator cache: %s", exc)

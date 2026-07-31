"""Explicit dispatch and fidelity policy types.

Backend selection and timing strictness are independent decisions.  Keeping
them in one small orchestration-owned module prevents the native adapter from
silently turning every Rust session into an acceptance run.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Literal

type DispatchBackend = Literal["auto", "rust", "python"]
type FidelityMode = Literal["normal", "strict"]


@dataclass(frozen=True, slots=True)
class DispatchPolicy:
    """Validated policy for one playback session."""

    backend: DispatchBackend = "auto"
    fidelity: FidelityMode = "normal"

    def __post_init__(self) -> None:
        if self.backend not in {"auto", "rust", "python"}:
            raise ValueError("dispatch backend must be 'auto', 'rust', or 'python'")
        if self.fidelity not in {"normal", "strict"}:
            raise ValueError("fidelity mode must be 'normal' or 'strict'")

    @property
    def strict_timing(self) -> bool:
        return self.fidelity == "strict"


@dataclass(frozen=True, slots=True)
class DispatchPlan:
    """Resolved implementation for one playback session."""

    backend: Literal["rust", "python"]
    reason: str
    fidelity: FidelityMode
    native_probe: Any | None = None

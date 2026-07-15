"""Background worker lifecycle management infrastructure."""

from __future__ import annotations

import contextlib
from concurrent.futures import Executor, Future
from dataclasses import dataclass
from enum import StrEnum
from typing import Any, Protocol


class ResourceState(StrEnum):
    OPEN = "open"
    CLOSING = "closing"
    CLOSED = "closed"
    FAILED = "failed"


class CleanupPolicy(StrEnum):
    RAISE = "raise"
    LOG_AND_CONTINUE = "log_and_continue"


@dataclass(frozen=True, slots=True)
class WorkerSnapshot:
    name: str
    phase: str
    closed: bool
    pending_count: int | None = None
    running_count: int | None = None
    state: str = "open"
    last_error: str | None = None

    def __post_init__(self) -> None:
        if self.state == "open" and self.closed:
            object.__setattr__(self, "state", "closed")


class BackgroundCleanupError(RuntimeError):
    def __init__(self, message: str, result: ScopeCloseResult) -> None:
        super().__init__(message)
        self.result = result


@dataclass(frozen=True, slots=True)
class ScopeCloseResult:
    phase: str
    snapshots: tuple[WorkerSnapshot, ...]
    errors: tuple[str, ...]


class BackgroundResource(Protocol):
    @property
    def name(self) -> str: ...

    @property
    def phase(self) -> str: ...

    def cancel(self) -> None: ...

    def close(self, *, wait: bool) -> None: ...

    def snapshot(self) -> WorkerSnapshot: ...


class BackgroundScope:
    def __init__(self, *, phase: str) -> None:
        self._phase = phase
        self._resources: list[BackgroundResource] = []
        self._retired_resources: list[BackgroundResource] = []
        self._closed = False

    def register(self, resource: BackgroundResource) -> BackgroundResource:
        self._resources.append(resource)
        return resource

    def retire(self, resource: BackgroundResource) -> None:
        if resource in self._resources:
            self._resources.remove(resource)
            self._retired_resources.append(resource)

    def cancel_all(self) -> list[Exception]:
        errors: list[Exception] = []
        for r in self._resources:
            try:
                r.cancel()
            except Exception as exc:
                errors.append(exc)
        for r in self._retired_resources:
            try:
                r.cancel()
            except Exception as exc:
                errors.append(exc)
        return errors

    def close_all(self, *, wait: bool) -> ScopeCloseResult:
        cancel_errors = self.cancel_all()
        errors: list[Exception] = list(cancel_errors)
        # Close in reverse registration order: active first, then retired.
        for r in reversed(self._resources):
            try:
                r.close(wait=wait)
            except Exception as exc:
                errors.append(exc)
        for r in reversed(self._retired_resources):
            try:
                r.close(wait=wait)
            except Exception as exc:
                errors.append(exc)
        self._closed = True

        snaps = self.snapshots()
        err_strs = tuple(str(e) for e in errors)
        result = ScopeCloseResult(
            phase=self._phase,
            snapshots=snaps,
            errors=err_strs,
        )

        # Drop closed resource refs so a long-lived scope does not pin executors/threads.
        self._resources.clear()
        self._retired_resources.clear()

        if errors:
            raise BackgroundCleanupError(
                f"Cleanup failures in phase '{self._phase}': {err_strs}",
                result=result,
            )
        return result

    def assert_closed(self) -> None:
        for r in self._resources:
            snap = r.snapshot()
            if not snap.closed:
                raise RuntimeError(f"Resource '{r.name}' is not closed")
        for r in self._retired_resources:
            snap = r.snapshot()
            if not snap.closed:
                raise RuntimeError(f"Retired resource '{r.name}' is not closed")

    def snapshots(self) -> tuple[WorkerSnapshot, ...]:
        snaps = []
        for r in self._resources:
            with contextlib.suppress(Exception):
                snaps.append(r.snapshot())
        for r in self._retired_resources:
            with contextlib.suppress(Exception):
                snaps.append(r.snapshot())
        return tuple(snaps)


class ExecutorResource:
    def __init__(self, *, name: str, phase: str, executor: Executor) -> None:
        self._name = name
        self._phase = phase
        self._executor = executor
        self._futures: list[Future[Any]] = []
        self._state = ResourceState.OPEN
        self._last_error = None

    @property
    def name(self) -> str:
        return self._name

    @property
    def phase(self) -> str:
        return self._phase

    def submit(self, fn: Any, /, *args: Any, **kwargs: Any) -> Future[Any]:
        if self._state is not ResourceState.OPEN:
            raise RuntimeError(f"ExecutorResource '{self._name}' is closed/closing")
        # Clean up done futures to prevent leaks
        self._futures = [f for f in self._futures if not f.done()]
        future = self._executor.submit(fn, *args, **kwargs)
        self._futures.append(future)
        return future

    def cancel(self) -> None:
        if self._state == ResourceState.OPEN:
            self._state = ResourceState.CLOSING
        for f in list(self._futures):
            if not f.done():
                f.cancel()

    def close(self, *, wait: bool) -> None:
        if self._state == ResourceState.CLOSED:
            return
        if self._state == ResourceState.OPEN:
            self._state = ResourceState.CLOSING
        self.cancel()
        if not wait:
            # Do not call executor shutdown with wait=False to avoid Python 3.14 manager-thread trap.
            # Simply request cancellation and transition to CLOSING.
            return
        try:
            try:
                self._executor.shutdown(wait=wait, cancel_futures=True)
            except TypeError:
                # Fallback if cancel_futures is not supported
                self._executor.shutdown(wait=wait)
            self._state = ResourceState.CLOSED
        except Exception as exc:
            self._state = ResourceState.FAILED
            self._last_error = str(exc)
            raise exc

    def snapshot(self) -> WorkerSnapshot:
        self._futures = [f for f in self._futures if not f.done()]
        pending = 0
        running = 0
        for f in self._futures:
            if f.running():
                running += 1
            elif not f.done():
                pending += 1
        return WorkerSnapshot(
            name=self._name,
            phase=self._phase,
            closed=self._state == ResourceState.CLOSED,
            pending_count=pending,
            running_count=running,
            state=self._state.value,
            last_error=self._last_error,
        )


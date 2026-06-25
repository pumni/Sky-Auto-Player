from __future__ import annotations

import pytest
from concurrent.futures import Executor, Future
from typing import Any
from sky_music.infrastructure.background import (
    BackgroundScope,
    ExecutorResource,
    WorkerSnapshot,
    BackgroundCleanupError,
)


class DummyExecutor(Executor):
    def __init__(self) -> None:
        self.shutdown_called = False
        self.shutdown_wait: bool | None = None
        self.shutdown_cancel_futures: bool | None = None
        self.submitted: list[tuple[Any, tuple[Any, ...], dict[str, Any]]] = []

    def submit(self, fn: Any, /, *args: Any, **kwargs: Any) -> Future[Any]:
        self.submitted.append((fn, args, kwargs))
        f: Future[Any] = Future()
        return f

    def shutdown(self, wait: bool = True, *, cancel_futures: bool = False) -> None:
        self.shutdown_called = True
        self.shutdown_wait = wait
        self.shutdown_cancel_futures = cancel_futures


class DummyResource:
    def __init__(self, name: str, phase: str) -> None:
        self._name = name
        self._phase = phase
        self.cancel_called = False
        self.close_called = False
        self.close_wait: bool | None = None
        self.closed_status = False

    @property
    def name(self) -> str:
        return self._name

    @property
    def phase(self) -> str:
        return self._phase

    def cancel(self) -> None:
        self.cancel_called = True

    def close(self, *, wait: bool) -> None:
        self.close_called = True
        self.close_wait = wait
        self.closed_status = True

    def snapshot(self) -> WorkerSnapshot:
        return WorkerSnapshot(
            name=self._name,
            phase=self._phase,
            closed=self.closed_status,
            pending_count=0,
            running_count=0,
        )


def test_executor_resource_close_wait_true() -> None:
    dummy_exec = DummyExecutor()
    res = ExecutorResource(name="test-exec", phase="picker", executor=dummy_exec)
    
    assert res.name == "test-exec"
    assert res.phase == "picker"
    
    res.close(wait=True)
    assert dummy_exec.shutdown_called
    assert dummy_exec.shutdown_wait is True
    assert dummy_exec.shutdown_cancel_futures is True
    assert res.snapshot().closed is True


def test_executor_resource_close_wait_false() -> None:
    dummy_exec = DummyExecutor()
    res = ExecutorResource(name="test-exec", phase="picker", executor=dummy_exec)
    res.close(wait=False)
    assert not dummy_exec.shutdown_called
    assert res.snapshot().state == "closing"



def test_executor_resource_cancel() -> None:
    dummy_exec = DummyExecutor()
    res = ExecutorResource(name="test-exec", phase="picker", executor=dummy_exec)
    
    f1 = res.submit(lambda: None)
    f2 = res.submit(lambda: None)
    
    res.cancel()
    
    assert f1.cancelled()
    assert f2.cancelled()


def test_executor_resource_close_idempotent() -> None:
    dummy_exec = DummyExecutor()
    res = ExecutorResource(name="test-exec", phase="picker", executor=dummy_exec)
    
    res.close(wait=True)
    assert dummy_exec.shutdown_called
    
    dummy_exec.shutdown_called = False
    res.close(wait=True)
    assert not dummy_exec.shutdown_called


def test_background_scope_close_all() -> None:
    scope = BackgroundScope(phase="picker")
    r1 = DummyResource("r1", "picker")
    r2 = DummyResource("r2", "picker")
    
    scope.register(r1)
    scope.register(r2)
    
    scope.close_all(wait=True)
    
    assert r1.cancel_called
    assert r1.close_called
    assert r1.close_wait is True
    assert r2.cancel_called
    assert r2.close_called
    assert r2.close_wait is True
    
    scope.assert_closed()


def test_background_scope_close_retired() -> None:
    scope = BackgroundScope(phase="picker")
    r1 = DummyResource("r1", "picker")
    r2 = DummyResource("r2", "picker")
    
    scope.register(r1)
    scope.register(r2)
    
    scope.retire(r1)
    
    scope.close_all(wait=True)
    
    assert r1.cancel_called
    assert r1.close_called
    assert r2.cancel_called
    assert r2.close_called
    
    scope.assert_closed()


def test_background_scope_close_order() -> None:
    scope = BackgroundScope(phase="picker")
    order: list[str] = []
    
    class OrderedResource(DummyResource):
        def cancel(self) -> None:
            super().cancel()
            order.append(f"cancel-{self.name}")
            
        def close(self, *, wait: bool) -> None:
            super().close(wait=wait)
            order.append(f"close-{self.name}")

    r1 = OrderedResource("r1", "picker")
    r2 = OrderedResource("r2", "picker")
    r3 = OrderedResource("r3", "picker")
    
    scope.register(r1)
    scope.register(r2)
    scope.register(r3)
    scope.retire(r2)
    
    scope.close_all(wait=True)
    
    # "Closing order should be deterministic: retire/cancel first, close in reverse registration order."
    # Wait, cancel_all cancels everything first.
    # The order of cancels will be: r1, r3 (from active), then r2 (from retired).
    # Then close in reverse registration order: active (r3, r1), then retired (r2).
    # Let's verify the exact close order: close-r3, close-r1, close-r2.
    close_indices = [order.index(f"close-{n}") for n in ("r3", "r1", "r2")]
    assert close_indices == sorted(close_indices)


def test_executor_resource_multiple_closes_wait_true() -> None:
    dummy_exec = DummyExecutor()
    res = ExecutorResource(name="test-exec", phase="picker", executor=dummy_exec)
    
    # First close with wait=False
    res.close(wait=False)
    assert not dummy_exec.shutdown_called
    assert res.snapshot().closed is False  # closed is False because final wait=True hasn't run/succeeded
    
    # Second close with wait=True must proceed to perform the wait
    res.close(wait=True)
    assert dummy_exec.shutdown_called
    assert dummy_exec.shutdown_wait is True
    assert res.snapshot().closed is True  # Now it is True


def test_background_scope_close_all_raises_errors() -> None:
    scope = BackgroundScope(phase="picker")
    
    class FaultyResource(DummyResource):
        def close(self, *, wait: bool) -> None:
            raise ValueError("Failed to close!")
            
    r = FaultyResource("faulty", "picker")
    scope.register(r)
    
    with pytest.raises(BackgroundCleanupError) as exc_info:
        scope.close_all(wait=True)
    assert len(exc_info.value.result.errors) == 1
    assert "Failed to close!" in exc_info.value.result.errors[0]


def test_executor_resource_snapshot_not_closed_on_failure() -> None:
    class FailingExecutor(DummyExecutor):
        def shutdown(self, wait: bool = True, *, cancel_futures: bool = False) -> None:
            raise RuntimeError("Shutdown failed!")
            
    res = ExecutorResource(name="test-exec", phase="picker", executor=FailingExecutor())
    
    with pytest.raises(RuntimeError, match="Shutdown failed!"):
        res.close(wait=True)
        
    snap = res.snapshot()
    assert snap.closed is False  # Did not close successfully
    assert snap.state == "failed"
    assert snap.last_error == "Shutdown failed!"


def sleep_job_module() -> int:
    import time
    time.sleep(0.05)
    return 42


def test_real_process_pool_executor_lifecycle() -> None:
    from concurrent.futures import ProcessPoolExecutor
    
    exec_obj = ProcessPoolExecutor(max_workers=1)
    res = ExecutorResource(name="real-process", phase="picker", executor=exec_obj)
    
    future = res.submit(sleep_job_module)
    
    # Simulating retirement flow: call cancel() instead of close(wait=False)
    res.cancel()
    assert res.snapshot().closed is False
    assert res.snapshot().state == "closing"
    
    # Close with wait=True must block until pool is fully shut down
    res.close(wait=True)
    assert res.snapshot().closed is True
    assert res.snapshot().state == "closed"
    assert future.done()
    # Retirement (res.cancel()) may cancel a job that the worker had not started yet — a valid
    # outcome of the closing flow. If it did run, the result must be 42.
    if future.cancelled():
        pass
    else:
        assert future.result() == 42


def test_executor_resource_state_transitions() -> None:
    dummy_exec = DummyExecutor()
    res = ExecutorResource(name="test-exec", phase="picker", executor=dummy_exec)
    
    assert res.snapshot().state == "open"
    assert res.snapshot().closed is False
    
    # close(wait=False) -> closing
    res.close(wait=False)
    assert res.snapshot().state == "closing"
    assert res.snapshot().closed is False
    
    # submit after close/cancel raises RuntimeError
    with pytest.raises(RuntimeError, match="closed/closing"):
        res.submit(lambda: None)

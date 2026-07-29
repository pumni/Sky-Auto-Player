import threading
import time
from unittest.mock import Mock

import pytest

from sky_music.orchestration.playback_supervisor import PlaybackSupervisor


def test_supervisor_exception_joins_dispatch_thread(monkeypatch):
    import sky_music.platform.win32.inputs as inputs
    
    # 1. Stub the dispatch loop to block, but allow quitting
    started_event = threading.Event()
    
    class MockDispatchLoop:
        def __init__(self):
            self.health_monitor = Mock()
            self.health_monitor.input_path_degraded = False
            self.sleeper = Mock()

        def run(self, state, command_source, focus_signal, progress_sink, total_time_us, command_event):
            started_event.set()
            while True:
                cmd = command_source.poll()
                if cmd in ("quit", "panic"):
                    break
                time.sleep(0.01)
            return "finished"
            
    mock_loop = MockDispatchLoop()
    
    # 2. Controls that raise after dispatch starts
    class FaultyControls:
        def __init__(self):
            self.called = False
            
        def poll(self):
            if started_event.is_set() and not self.called:
                self.called = True
                raise RuntimeError("Simulated control error")
            return
            
    telemetry_mock = Mock()
    telemetry_mock.runtime_options = {}

    from sky_music.infrastructure.timing import SleepPolicy
    
    supervisor = PlaybackSupervisor(
        controls=FaultyControls(),
        focus_guard=Mock(),
        require_focus=False,
        renderer=Mock(),
        telemetry=telemetry_mock,
        sleep_policy=SleepPolicy(),
        clock=Mock(),
        sleeper=Mock(),
        song_name="Test",
        enable_event_wait=True,
    )
    
    events = []
    
    original_join = threading.Thread.join
    def mock_join(self, timeout=None):
        if self.name == "sky-music-dispatch":
            events.append("join")
        return original_join(self, timeout)
        
    monkeypatch.setattr(threading.Thread, "join", mock_join)
    
    original_close = inputs.close_handle
    def mock_close(handle):
        events.append("close")
        if original_close:
            try:
                return original_close(handle)
            except Exception:
                pass
        return None
        
    monkeypatch.setattr(inputs, "close_handle", mock_close)
    
    state_mock = Mock()
    state_mock.elapsed_snapshot_us.return_value = (0, False)
    coordinator_mock = Mock()
    
    # Run the supervisor - it should start the dispatch thread and then immediately crash 
    # in the control loop, which should trigger the structured shutdown.
    with pytest.raises(RuntimeError, match="Simulated control error"):
        supervisor.run(
            dispatch_loop=mock_loop,  # type: ignore
            coordinator=coordinator_mock,  # type: ignore
            state=state_mock,  # type: ignore
            total_time_us=1000,
            use_dispatch_thread=True
        )
        
    # Assert ordering: join attempt happened before close
    assert "join" in events
    assert "close" in events
    assert events.index("join") < events.index("close")

def test_shutdown_timeout_resource_safe_when_dispatch_thread_stuck(monkeypatch):
    import sky_music.platform.win32.inputs as inputs
    
    events = []
    stop_event = threading.Event()
    
    # 1. Stub the dispatch loop to hang conditionally
    class StuckDispatchLoop:
        def __init__(self):
            self.health_monitor = Mock()
            self.health_monitor.input_path_degraded = False
            self.sleeper = Mock()

        def run(self, state, command_source, focus_signal, progress_sink, total_time_us, command_event):
            stop_event.wait(5.0) # Wait up to 5s to avoid permanent hang if event is lost
            return "finished"
            
    # 2. Controls that raise immediately to trigger shutdown
    class FaultyControls:
        def poll(self):
            raise RuntimeError("Simulated control error to trigger shutdown")
            
    telemetry_mock = Mock()
    telemetry_mock.runtime_options = {}

    from sky_music.infrastructure.timing import SleepPolicy
    
    supervisor = PlaybackSupervisor(
        controls=FaultyControls(),
        focus_guard=Mock(),
        require_focus=False,
        renderer=Mock(),
        telemetry=telemetry_mock,
        sleep_policy=SleepPolicy(),
        clock=Mock(),
        sleeper=Mock(),
        song_name="Test",
        enable_event_wait=True,
    )
    
    # Mock join to return immediately, but thread.is_alive() remains True
    original_join = threading.Thread.join
    def mock_join(self, timeout=None):
        if self.name == "sky-music-dispatch":
            events.append("join")
            return None # simulate timeout
        return original_join(self, timeout)
        
    monkeypatch.setattr(threading.Thread, "join", mock_join)
    
    original_is_alive = threading.Thread.is_alive
    def mock_is_alive(self):
        if self.name == "sky-music-dispatch":
            events.append("is_alive")
            return True # Thread is still stuck!
        return original_is_alive(self)
        
    monkeypatch.setattr(threading.Thread, "is_alive", mock_is_alive)
    
    original_close = inputs.close_handle
    def mock_close(handle):
        events.append("close")
        if original_close:
            return original_close(handle)
        return None
        
    monkeypatch.setattr(inputs, "close_handle", mock_close)
    
    # Ensure supervisor returns or raises instead of hanging on sleep(5)
    # the test must not sleep 5 seconds!
    def mock_sleep(s):
        pass
    monkeypatch.setattr(time, "sleep", mock_sleep)
    
    state_mock = Mock()
    state_mock.elapsed_snapshot_us.return_value = (0, False)
    
    try:
        supervisor.run(
            dispatch_loop=StuckDispatchLoop(),  # type: ignore
            coordinator=Mock(),  # type: ignore
            state=state_mock,  # type: ignore
            total_time_us=1000,
            use_dispatch_thread=True
        )
    except Exception:
        pass # Depending on if it propagates the error or raises a new timeout one
    finally:
        stop_event.set()
        
    assert "join" in events
    assert "is_alive" in events
    # If thread is still alive, we must NOT close handles or declare safe!
    assert "close" not in events
    # Lifecycle contract (fix for the SHUTDOWN_TIMEOUT teardown race): the supervisor must
    # publish the live thread handle AND mark it NOT-terminated so the engine finally block
    # can skip clear_array_cache/close/collect_gc_without having to re-derive liveness.
    assert supervisor.dispatch_thread is not None
    assert supervisor.dispatch_thread_terminated is False


def test_supervisor_direct_mode_publishes_no_thread_handle() -> None:
    """Direct (non-threaded) mode publishes ``dispatch_thread=None, terminated=True``.

    Lets the engine finally block run the full teardown path without worrying about a
    dispatch thread that does not exist.
    """
    from unittest.mock import Mock

    from sky_music.infrastructure.timing import SleepPolicy
    from sky_music.orchestration.core.ports import PLAYBACK_FINISHED

    class ImmediateDispatchLoop:
        def __init__(self) -> None:
            self.sleeper = Mock()

        def run(self, *args, **kwargs) -> str:
            return PLAYBACK_FINISHED

    supervisor = PlaybackSupervisor(
        controls=None,
        focus_guard=Mock(),
        require_focus=False,
        renderer=Mock(),
        telemetry=Mock(spec=["runtime_options", "record_runtime_options"]),
        sleep_policy=SleepPolicy(),
        clock=Mock(),
        sleeper=Mock(),
        song_name="direct",
    )
    supervisor.telemetry.runtime_options = {}
    result = supervisor.run(
        dispatch_loop=ImmediateDispatchLoop(),  # type: ignore
        coordinator=Mock(),  # type: ignore
        state=Mock(),  # type: ignore
        total_time_us=1000,
        use_dispatch_thread=False,
    )
    assert result == PLAYBACK_FINISHED
    assert supervisor.dispatch_thread is None
    assert supervisor.dispatch_thread_terminated is True


def test_engine_skips_teardown_on_shutdown_timeout(monkeypatch) -> None:
    """Engine finally block must NOT clear the INPUT cache / close the realtime sleeper /
    drop per-song state / gc.collect() when PlaybackSupervisor reports the dispatch thread
    is still alive (PLAYBACK_SHUTDOWN_TIMEOUT branch).

    Regression guard for the lifecycle teardown race in review of main@7c548527 §1: the
    previous code used ``getattr(supervisor, "dispatch_thread", None)`` which always returned
    None, so the engine tore down shared resources while the dispatch thread was still using
    them.
    """
    from unittest.mock import Mock

    from sky_music.domain import Song
    from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
    from sky_music.infrastructure.backend import DryRunBackend
    from sky_music.orchestration.engine import PLAYBACK_SHUTDOWN_TIMEOUT, PlaybackEngine
    from sky_music.platform.win32 import inputs

    # A stuck supervisor fake: returns PLAYBACK_SHUTDOWN_TIMEOUT and reports the thread as
    # still alive, so the engine finally block should skip teardown.
    class StuckSupervisor:
        def __init__(self, *args, **kwargs) -> None:
            self.dispatch_thread = Mock()
            self.dispatch_thread.is_alive.return_value = True
            self.dispatch_thread_terminated = False

        def run(self, **kwargs) -> str:
            return PLAYBACK_SHUTDOWN_TIMEOUT

    monkeypatch.setattr(
        "sky_music.orchestration.engine.PlaybackSupervisor", StuckSupervisor
    )

    clear_calls: list[int] = []
    orig_clear = inputs.clear_array_cache

    def spy_clear() -> int:
        clear_calls.append(len(clear_calls))
        return orig_clear()

    monkeypatch.setattr(inputs, "clear_array_cache", spy_clear)

    # Build engine in threaded-mode-equivalent config but with a backend that needs no Windows.
    # ``_should_use_dispatch_thread`` is patched to True so we occupy the threaded path; the
    # StuckSupervisor stands in before any real thread starts.
    engine = PlaybackEngine(
        song=Song(name="lifecycle", notes=()),
        actions=(
            KeyAction(
                kind="down",  # type: ignore[arg-type]
                scan_codes=(ScanCode(21),),
                at_us=Microseconds(0),
                reason="lifecycle-test",
            ),
            KeyAction(
                kind="up",  # type: ignore[arg-type]
                scan_codes=(ScanCode(21),),
                at_us=Microseconds(100_000),
                reason="lifecycle-test",
            ),
        ),
        backend=DryRunBackend(),
        controls=None,
        renderer=None,
        require_focus=False,
        use_dispatch_thread=True,
        lead_cache_path=None,
    )
    monkeypatch.setattr(engine, "_should_use_dispatch_thread", lambda: True)

    result = engine.play()
    assert result == PLAYBACK_SHUTDOWN_TIMEOUT

    # The teardown path MUST NOT have been entered: clear_array_cache must not be called and
    # the engine must still hold its runtime_schedule (poisoned-but-intact state — a follow-up
    # play() would rebuild; the orphaned dispatch thread can still read its coordinator).
    assert clear_calls == [], (
        f"clear_array_cache must NOT run on PLAYBACK_SHUTDOWN_TIMEOUT; got {clear_calls}"
    )
    assert engine.runtime_schedule is not None, "runtime_schedule must be retained on stuck path"
    assert engine._runtime_coordinator is not None, "coordinator must be retained on stuck path"



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


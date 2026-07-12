import sys
import threading
import time

from sky_music.watchdog import SKY_15_SCAN_CODES


def test_watchdog_panics_on_timeout(monkeypatch):
    """Test that watchdog panics (calls release_all) when heartbeat times out."""
    # We can invoke the watchdog as a subprocess and mock send_scan_code_batch?
    # No, it's easier to run it via subprocess and capture stdout/stderr, but it only calls sendInput.
    # To test the logic cleanly, we can test the module by overriding sys.stdin and send_scan_code_batch
    # in the current process. But it's an infinite loop until EOF or timeout.
    
    import sky_music.watchdog
    
    class DummyStdin:
        def __init__(self):
            self.lock = threading.Lock()
            self.buf = b''
            self.closed = False
            self.cond = threading.Condition(self.lock)
            
        def write(self, data: bytes):
            with self.cond:
                self.buf += data
                self.cond.notify_all()
                
        def close(self):
            with self.cond:
                self.closed = True
                self.cond.notify_all()
                
        def read(self, n: int) -> bytes:
            with self.cond:
                while not self.buf and not self.closed:
                    self.cond.wait()
                if self.buf:
                    ret = self.buf[:n]
                    self.buf = self.buf[n:]
                    return ret
                return b''

    dummy_stdin = DummyStdin()
    
    class DummyBuffer:
        def read(self, n: int) -> bytes:
            return dummy_stdin.read(n)
            
    class FakeStdin:
        buffer = DummyBuffer()

    monkeypatch.setattr(sys, "stdin", FakeStdin())
    
    # We will patch TIMEOUT_SEC to be very short to speed up the test
    monkeypatch.setattr(sky_music.watchdog, "TIMEOUT_SEC", 0.1)
    
    panicked_args = []
    def mock_send(scan_codes, key_up):
        panicked_args.append((scan_codes, key_up))
        
    monkeypatch.setattr(sky_music.watchdog, "send_scan_code_batch", mock_send)
    
    # Start the watchdog main in a thread
    t = threading.Thread(target=sky_music.watchdog.main)
    t.start()
    
    # Send a heartbeat
    dummy_stdin.write(b'\x00')
    time.sleep(0.05)
    
    # Send another heartbeat
    dummy_stdin.write(b'\x00')
    time.sleep(0.05)
    
    # Now just wait for timeout (0.1s)
    t.join(timeout=2.0)
    
    # It should have panicked
    assert not t.is_alive()
    assert len(panicked_args) == 1
    assert set(panicked_args[0][0]) == set(SKY_15_SCAN_CODES)
    assert panicked_args[0][1] is True


def test_watchdog_clean_exit_on_eof(monkeypatch):
    """Test that watchdog exits cleanly (no panic) on EOF."""
    import sky_music.watchdog
    
    class DummyStdin:
        def __init__(self):
            self.lock = threading.Lock()
            self.buf = b''
            self.closed = False
            self.cond = threading.Condition(self.lock)
            
        def write(self, data: bytes):
            with self.cond:
                self.buf += data
                self.cond.notify_all()
                
        def close(self):
            with self.cond:
                self.closed = True
                self.cond.notify_all()
                
        def read(self, n: int) -> bytes:
            with self.cond:
                while not self.buf and not self.closed:
                    self.cond.wait()
                if self.buf:
                    ret = self.buf[:n]
                    self.buf = self.buf[n:]
                    return ret
                return b''

    dummy_stdin = DummyStdin()
    
    class DummyBuffer:
        def read(self, n: int) -> bytes:
            return dummy_stdin.read(n)
            
    class FakeStdin:
        buffer = DummyBuffer()

    monkeypatch.setattr(sys, "stdin", FakeStdin())
    monkeypatch.setattr(sky_music.watchdog, "TIMEOUT_SEC", 0.5)
    
    panicked = False
    def mock_send(scan_codes, key_up):
        nonlocal panicked
        panicked = True
        
    monkeypatch.setattr(sky_music.watchdog, "send_scan_code_batch", mock_send)
    
    t = threading.Thread(target=sky_music.watchdog.main)
    t.start()
    
    # Send a heartbeat
    dummy_stdin.write(b'\x00')
    time.sleep(0.05)
    
    # Close stdin (EOF)
    dummy_stdin.close()
    
    # Should exit immediately without panicking
    t.join(timeout=1.0)
    assert not t.is_alive()
    assert not panicked

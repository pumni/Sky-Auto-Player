import sys

import pytest

from sky_music.platform.win32 import inputs


@pytest.mark.skipif(sys.platform != "win32", reason="Requires Win32")
def test_win32_event_prototypes_are_declared():
    kernel32 = inputs.kernel32
    import ctypes
    from ctypes import wintypes
    
    # CreateEventW
    assert kernel32.CreateEventW.argtypes is not None, "CreateEventW argtypes missing"
    assert kernel32.CreateEventW.restype in (wintypes.HANDLE, ctypes.c_void_p), "CreateEventW restype must be HANDLE"
    
    # SetEvent
    assert kernel32.SetEvent.argtypes is not None, "SetEvent argtypes missing"
    assert kernel32.SetEvent.restype == wintypes.BOOL, "SetEvent restype must be BOOL"
    
    # WaitForMultipleObjects
    assert kernel32.WaitForMultipleObjects.argtypes is not None, "WaitForMultipleObjects argtypes missing"
    assert kernel32.WaitForMultipleObjects.restype == wintypes.DWORD, "WaitForMultipleObjects restype must be DWORD"


@pytest.mark.skipif(sys.platform != "win32", reason="Requires Win32")
def test_wait_for_multiple_objects_handles_wait_failed(monkeypatch):
    
    # Patch the real kernel32 API in the inputs module
    def mock_wait_for_multiple_objects(count, handles, wait_all, timeout):
        # Return WAIT_FAILED
        return 0xFFFFFFFF
        
    monkeypatch.setattr(inputs.kernel32, "WaitForMultipleObjects", mock_wait_for_multiple_objects)
    
    # Call the wrapper with dummy handles
    # WAIT_FAILED should be treated as None (not woken by event)
    res = inputs.wait_for_multiple_objects((1234, 5678), 100)
    assert res is None, "wait_for_multiple_objects must return None when WAIT_FAILED is returned"

import ctypes
from ctypes import wintypes

from sky_music.infrastructure.hotkey_hook import (
    KBDLLHOOKSTRUCT,
    SKY_PLAYER_SIGNATURE,
    WM_KEYDOWN,
    HotkeyHook,
)
from sky_music.infrastructure.hotkeys import HotkeyBinding, PlaybackControls


def test_hotkey_hook_filters_signature(monkeypatch):
    controls = PlaybackControls(
        pause=HotkeyBinding("f8", 0x77),
        skip=HotkeyBinding("f9", 0x78),
        quit=HotkeyBinding("f10", 0x79),
        refocus=HotkeyBinding("f6", 0x75),
        panic=HotkeyBinding("backspace", 0x08, ctrl=True, alt=True),
    )
    hook = HotkeyHook(controls)
    
    # Mock user32.CallNextHookEx
    called_next = []
    def mock_call_next(hook_id, nCode, wParam, lParam):
        called_next.append(True)
        return 0
        
    import sky_music.infrastructure.hotkey_hook
    monkeypatch.setattr(sky_music.infrastructure.hotkey_hook.user32, "CallNextHookEx", mock_call_next)
    
    # Create info with signature
    info = KBDLLHOOKSTRUCT()
    info.vkCode = 0x77 # F8
    
    # Set signature
    ULONG_PTR = ctypes.POINTER(wintypes.ULONG)
    extra_val = wintypes.ULONG(SKY_PLAYER_SIGNATURE)
    info.dwExtraInfo = ctypes.cast(ctypes.pointer(extra_val), ULONG_PTR)
    
    # Send KeyDown for F8. It should be skipped due to signature.
    res = hook._hook_proc(0, WM_KEYDOWN, ctypes.pointer(info))
    
    assert res == 0 # Returns what CallNextHookEx returns
    assert called_next == [True]
    assert hook.poll() is None

def test_hotkey_hook_swallows_hotkey(monkeypatch):
    controls = PlaybackControls(
        pause=HotkeyBinding("f8", 0x77),
        skip=HotkeyBinding("f9", 0x78),
        quit=HotkeyBinding("f10", 0x79),
        refocus=HotkeyBinding("f6", 0x75),
        panic=HotkeyBinding("backspace", 0x08, ctrl=True, alt=True),
    )
    hook = HotkeyHook(controls)
    
    called_next = []
    def mock_call_next(hook_id, nCode, wParam, lParam):
        called_next.append(True)
        return 0
        
    import sky_music.infrastructure.hotkey_hook
    monkeypatch.setattr(sky_music.infrastructure.hotkey_hook.user32, "CallNextHookEx", mock_call_next)
    
    info = KBDLLHOOKSTRUCT()
    info.vkCode = 0x77 # F8
    info.dwExtraInfo = ctypes.cast(0, ctypes.POINTER(wintypes.ULONG))
    
    res = hook._hook_proc(0, WM_KEYDOWN, ctypes.pointer(info))
    
    assert res == 1 # Swallowed!
    assert not called_next
    assert hook.poll() == "pause"
    assert hook.poll() is None

def test_hotkey_hook_passes_unrelated_key(monkeypatch):
    controls = PlaybackControls(
        pause=HotkeyBinding("f8", 0x77),
        skip=HotkeyBinding("f9", 0x78),
        quit=HotkeyBinding("f10", 0x79),
        refocus=HotkeyBinding("f6", 0x75),
        panic=HotkeyBinding("backspace", 0x08, ctrl=True, alt=True),
    )
    hook = HotkeyHook(controls)
    
    called_next = []
    def mock_call_next(hook_id, nCode, wParam, lParam):
        called_next.append(True)
        return 0
        
    import sky_music.infrastructure.hotkey_hook
    monkeypatch.setattr(sky_music.infrastructure.hotkey_hook.user32, "CallNextHookEx", mock_call_next)
    
    info = KBDLLHOOKSTRUCT()
    info.vkCode = 0x41 # 'A'
    info.dwExtraInfo = ctypes.cast(0, ctypes.POINTER(wintypes.ULONG))
    
    res = hook._hook_proc(0, WM_KEYDOWN, ctypes.pointer(info))
    
    assert res == 0 # Not swallowed
    assert called_next == [True]
    assert hook.poll() is None

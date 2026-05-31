from typing import Protocol

class FocusGuard(Protocol):
    def is_active(self) -> bool:
        """Returns True if the target game window is currently active/focused."""
        ...
        
    def focus(self) -> bool:
        """Attempts to bring the target game window to the foreground."""
        ...

class NoopFocusGuard:
    """Mock FocusGuard useful for dry-runs, tests, and non-Windows environments."""
    def is_active(self) -> bool:
        return True
        
    def focus(self) -> bool:
        return True

class Win32SkyFocusGuard:
    """Windows-specific implementation using the custom inputs user32 wrapper."""
    def is_active(self) -> bool:
        from sky_music.platform.win32 import inputs
        return bool(inputs.is_sky_active())
        
    def focus(self) -> bool:
        from sky_music.platform.win32 import inputs
        return bool(inputs.focusWindow())

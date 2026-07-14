from __future__ import annotations

from textual.message import Message


class PickerActionRequested(Message):
    """Fired when a component wants the app to perform a picker action.
    
    This decouples child components (like the song table or the footer) 
    from the main SkyPickerApp host, allowing them to request actions 
    (like 'open_profile' or 'toggle_theme') without directly inspecting 
    `self.app` and calling `getattr(self.app, 'action_...')`.
    """
    def __init__(self, action: str) -> None:
        super().__init__()
        self.action = action

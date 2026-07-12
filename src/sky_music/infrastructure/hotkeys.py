from dataclasses import dataclass, field
from typing import Any

from sky_music.layouts import SKY_15_KEY_MAP as key_maps
from sky_music.layouts import VK_CODES
from sky_music.platform.win32.inputs import is_virtual_key_down

VK_CONTROL = 0x11
VK_SHIFT = 0x10
VK_MENU = 0x12
VK_ESCAPE = 0x1B
VK_SPACE = 0x20
VK_ENTER = 0x0D
VK_TAB = 0x09
VK_BACKSPACE = 0x08

SPECIAL_HOTKEY_CODES = {
    "esc": VK_ESCAPE,
    "escape": VK_ESCAPE,
    "space": VK_SPACE,
    "enter": VK_ENTER,
    "return": VK_ENTER,
    "tab": VK_TAB,
    "backspace": VK_BACKSPACE,
}

VK_CODE_BY_KEY_NAME = {
    **VK_CODES,
    ";": 0xBA,
    ",": 0xBC,
    ".": 0xBE,
    "/": 0xBF,
}

@dataclass(frozen=True, slots=True)
class HotkeyBinding:
    name: str
    key_code: int
    ctrl: bool = False
    alt: bool = False
    shift: bool = False

    @property
    def display(self) -> str:
        parts = []
        if self.ctrl:
            parts.append("Ctrl")
        if self.alt:
            parts.append("Alt")
        if self.shift:
            parts.append("Shift")
        parts.append(self.name.upper() if len(self.name) == 1 else self.name)
        return "+".join(parts)

    @property
    def has_modifier(self) -> bool:
        return self.ctrl or self.alt or self.shift

@dataclass(slots=True)
class PlaybackControls:
    pause: HotkeyBinding
    skip: HotkeyBinding
    quit: HotkeyBinding
    refocus: HotkeyBinding
    panic: HotkeyBinding
    enabled: bool = True
    use_ll_hook: bool = False
    _was_down: dict[str, bool] = field(default_factory=dict)
    _hook: Any = field(default=None, init=False)

    def __post_init__(self) -> None:
        if self.use_ll_hook:
            try:
                from sky_music.infrastructure.hotkey_hook import HotkeyHook
                self._hook = HotkeyHook(self)
                self._hook.start()
            except Exception:
                self._hook = None

    def stop_hook(self) -> None:
        if self._hook:
            self._hook.stop()
            self._hook = None

    def hint(self) -> str:
        if not self.enabled:
            return "hotkeys disabled"
        return (
            f"{self.pause.display} pause/resume | "
            f"{self.skip.display} skip | "
            f"{self.quit.display} quit | "
            f"{self.refocus.display} refocus Sky | "
            f"{self.panic.display} panic release"
        )

    def poll(self) -> str | None:
        if not self.enabled:
            return None
            
        if self._hook is not None:
            # The hook's own thread will push events into its queue
            action = self._hook.poll()
            if action is not None:
                return action
                
        for action, hotkey in (
            ("quit", self.quit),
            ("skip", self.skip),
            ("pause", self.pause),
            ("refocus", self.refocus),
            ("panic", self.panic),
        ):
            is_down = is_hotkey_down(hotkey)

            if is_down and not self._was_down.get(action, False):
                self._was_down[action] = True
                return action

            self._was_down[action] = is_down

        return None

def is_hotkey_down(hotkey: HotkeyBinding) -> bool:
    """Check if a hotkey is currently pressed.

    Required modifiers must be held; extra modifiers are ignored unless
    the hotkey itself has no modifiers (to avoid false positives with
    Ctrl+something accidentally triggering plain-key hotkeys).
    """
    ctrl_down = is_virtual_key_down(VK_CONTROL)
    alt_down = is_virtual_key_down(VK_MENU)
    shift_down = is_virtual_key_down(VK_SHIFT)

    # Required modifiers must be held
    if hotkey.ctrl and not ctrl_down:
        return False
    if hotkey.alt and not alt_down:
        return False
    if hotkey.shift and not shift_down:
        return False

    # For plain (no-modifier) hotkeys, require that no modifier is held
    # to prevent Ctrl+F8 accidentally triggering the plain F8 hotkey.
    if not hotkey.has_modifier and (ctrl_down or alt_down or shift_down):
        return False

    return is_virtual_key_down(hotkey.key_code)

def parse_hotkey(value: str) -> HotkeyBinding:
    raw = value.strip()
    if not raw:
        raise ValueError("hotkey cannot be empty")

    tokens = [token.strip().casefold() for token in raw.replace("-", "+").split("+") if token.strip()]
    ctrl = False
    alt = False
    shift = False
    key_token = None

    for token in tokens:
        if token in {"ctrl", "control", "ctl"}:
            ctrl = True
        elif token == "alt":
            alt = True
        elif token == "shift":
            shift = True
        else:
            if key_token is not None:
                raise ValueError(f"invalid hotkey {value!r}: too many key tokens")
            key_token = token

    if key_token is None:
        raise ValueError(f"invalid hotkey {value!r}: missing key")

    if key_token.startswith("f") and key_token[1:].isdigit():
        index = int(key_token[1:])
        if 1 <= index <= 24:
            return HotkeyBinding(f"F{index}", 0x70 + index - 1, ctrl=ctrl, alt=alt, shift=shift)
        raise ValueError(f"unsupported function key: {key_token}")

    if key_token in SPECIAL_HOTKEY_CODES:
        display_name = "Esc" if key_token in {"esc", "escape"} else key_token.title()
        return HotkeyBinding(display_name, SPECIAL_HOTKEY_CODES[key_token], ctrl=ctrl, alt=alt, shift=shift)

    if len(key_token) == 1:
        key_code = VK_CODE_BY_KEY_NAME.get(key_token)
        if key_code is None and "a" <= key_token <= "z":
            key_code = ord(key_token.upper())
        if key_code is not None:
            return HotkeyBinding(key_token, key_code, ctrl=ctrl, alt=alt, shift=shift)

    raise ValueError(f"unsupported hotkey: {value!r}")

def hotkey_conflicts_with_note_keys(hotkey: HotkeyBinding) -> bool:
    if hotkey.has_modifier:
        return False
    return hotkey.name.casefold() in {mapped_key.casefold() for mapped_key in key_maps.values()}

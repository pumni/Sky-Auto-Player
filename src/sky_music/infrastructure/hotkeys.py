from dataclasses import dataclass, field

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
    _was_down: dict[str, bool] = field(default_factory=dict)

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

        # Snapshot the three modifier virtual-key states ONCE per poll tick instead of
        # once per hotkey. Review of main@7c548527 §1.5: every ``is_hotkey_down`` call
        # re-queried Ctrl/Alt/Shift via ``GetAsyncKeyState``, so a 5-hotkey poll issued up
        # to 20 Win32 calls/tick (≈ 2 000 calls/s during playback at the 10 ms cadence).
        # The snapshotted modifiers are passed into ``_eval_hotkey_with_modifiers`` for
        # each binding, leaving exactly 3 + 5 = 8 calls/tick — a 60 % syscall reduction at
        # no behavioural or safety-policy cost (the modifier flags are READ-ONLY state and
        # cannot change between bindings within a single GetAsyncKeyState-coherent tick).
        ctrl_down = is_virtual_key_down(VK_CONTROL)
        alt_down = is_virtual_key_down(VK_MENU)
        shift_down = is_virtual_key_down(VK_SHIFT)

        for action, hotkey in (
            ("quit", self.quit),
            ("skip", self.skip),
            ("pause", self.pause),
            ("refocus", self.refocus),
            ("panic", self.panic),
        ):
            is_down = _eval_hotkey_with_modifiers(
                hotkey, ctrl_down=ctrl_down, alt_down=alt_down, shift_down=shift_down
            )

            if is_down and not self._was_down.get(action, False):
                self._was_down[action] = True
                return action

            self._was_down[action] = is_down

        return None

def _eval_hotkey_with_modifiers(
    hotkey: HotkeyBinding, *, ctrl_down: bool, alt_down: bool, shift_down: bool
) -> bool:
    """Evaluate a hotkey against caller-snapshotted modifier states.

    Same gating policy as ``is_hotkey_down``:
      * Required modifiers must be held.
      * For plain (no-modifier) hotkeys, ALL modifier flags must be clear — prevents
        Ctrl+F8 from accidentally firing a plain-F8 binding.
    """
    if hotkey.ctrl and not ctrl_down:
        return False
    if hotkey.alt and not alt_down:
        return False
    if hotkey.shift and not shift_down:
        return False
    if not hotkey.has_modifier and (ctrl_down or alt_down or shift_down):
        return False
    return is_virtual_key_down(hotkey.key_code)

def is_hotkey_down(hotkey: HotkeyBinding) -> bool:
    """Check if a hotkey is currently pressed.

    Required modifiers must be held; extra modifiers are ignored unless
    the hotkey itself has no modifiers (to avoid false positives with
    Ctrl+something accidentally triggering plain-key hotkeys).

    Single-call entry point for one-shot callers (e.g. the debug-toggle poll in
    ``playback_app``). The per-tick ``PlaybackControls.poll`` path snapshots every
    modifier once and routes through ``_eval_hotkey_with_modifiers`` instead of this
    function (review of main@7c548527 §1.5: removes ~ 60 % of redundant GetAsyncKeyState
    syscalls during playback). Out-of-loop callers who evaluate multiple hotkeys should
    prefer snapshotting + ``_eval_hotkey_with_modifiers`` themselves.
    """
    ctrl_down = is_virtual_key_down(VK_CONTROL)
    alt_down = is_virtual_key_down(VK_MENU)
    shift_down = is_virtual_key_down(VK_SHIFT)
    return _eval_hotkey_with_modifiers(
        hotkey, ctrl_down=ctrl_down, alt_down=alt_down, shift_down=shift_down
    )

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

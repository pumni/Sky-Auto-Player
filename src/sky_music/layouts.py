from sky_music.domain import InstrumentProfile, NoteKey

# Physical layout of the 15-key keyboard
# Row 1: Y U I O P
# Row 2: H J K L ;
# Row 3: N M , . /

VK_CODES = {
    'y': 0x59, 'u': 0x55, 'i': 0x49, 'o': 0x4F, 'p': 0x50,
    'h': 0x48, 'j': 0x4A, 'k': 0x4B, 'l': 0x4C, ';': 0xBA,
    'n': 0x4E, 'm': 0x4D, ',': 0xBC, '.': 0xBE, '/': 0xBF,
}

PHYSICAL_SCAN_CODES = {
    'y': 0x15, 'u': 0x16, 'i': 0x17, 'o': 0x18, 'p': 0x19,
    'h': 0x23, 'j': 0x24, 'k': 0x25, 'l': 0x26, ';': 0x27,
    'n': 0x31, 'm': 0x32, ',': 0x33, '.': 0x34, '/': 0x35,
}

_USER32 = None


def _map_virtual_key(vk: int) -> int:
    global _USER32
    if _USER32 is None:
        import ctypes
        _USER32 = ctypes.WinDLL("user32", use_last_error=True)
    return int(_USER32.MapVirtualKeyW(vk, 0))

SKY_15_KEY_MAP = {
    'Key0': 'y', 'Key1': 'u', 'Key2': 'i', 'Key3': 'o', 'Key4': 'p',
    'Key5': 'h', 'Key6': 'j', 'Key7': 'k', 'Key8': 'l', 'Key9': ';',
    'Key10': 'n', 'Key11': 'm', 'Key12': ',', 'Key13': '.', 'Key14': '/',
    # Legacy compatibility
    '1Key0': 'y', '1Key1': 'u', '1Key2': 'i', '1Key3': 'o', '1Key4': 'p',
    '1Key5': 'h', '1Key6': 'j', '1Key7': 'k', '1Key8': 'l', '1Key9': ';',
    '1Key10': 'n', '1Key11': 'm', '1Key12': ',', '1Key13': '.', '1Key14': '/',
    '2Key0': 'y', '2Key1': 'u', '2Key2': 'i', '2Key3': 'o', '2Key4': 'p',
    '2Key5': 'h', '2Key6': 'j', '2Key7': 'k', '2Key8': 'l', '2Key9': ';',
    '2Key10': 'n', '2Key11': 'm', '2Key12': ',', '2Key13': '.', '2Key14': '/',
    '3Key0': 'y', '3Key1': 'u', '3Key2': 'i', '3Key3': 'o', '3Key4': 'p',
    '3Key5': 'h', '3Key6': 'j', '3Key7': 'k', '3Key8': 'l', '3Key9': ';',
    '3Key10': 'n', '3Key11': 'm', '3Key12': ',', '3Key13': '.', '3Key14': '/',
}

SKY_15_KEY_PROFILE = InstrumentProfile(
    name="sky_15_key",
    note_count=15,
    key_map={NoteKey(k): v for k, v in SKY_15_KEY_MAP.items()}
)

class NoteResolver:
    def resolve_scan_code(self, note_key: NoteKey, mode: str = "physical") -> int:
        raise NotImplementedError

class DefaultNoteResolver(NoteResolver):
    def __init__(self, profile: InstrumentProfile):
        self.profile = profile

    def resolve_scan_code(self, note_key: NoteKey, mode: str = "physical") -> int:
        char = self.profile.key_map.get(note_key)
        if not char:
            return 0
        if mode == "physical":
            return PHYSICAL_SCAN_CODES.get(char, 0)
        # "mapped" mode: translate the virtual key to a scan code for the OS keyboard
        # layout via MapVirtualKey, falling back to the fixed physical scan code when it is
        # unavailable (non-Windows / test environments). Previously this lived in a separate
        # Win32NoteResolver; consolidated here so there is a single resolver everywhere.
        vk = VK_CODES.get(char)
        if vk is not None:
            try:
                sc = _map_virtual_key(vk)
                if sc:
                    return int(sc)
            except (AttributeError, OSError):
                pass
        return PHYSICAL_SCAN_CODES.get(char, 0)

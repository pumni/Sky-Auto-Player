import sys
from pathlib import Path

src_dir = Path(__file__).parent.parent / "src"
sys.path.insert(0, str(src_dir))

from sky_music.layouts import SKY_15_KEY_PROFILE, SKY_15_KEY_MAP

def test_layout_completeness():
    """Test that default 15-key profile maps exactly 15 unique key indexes correctly."""
    key_map = SKY_15_KEY_PROFILE.key_map
    
    # Extract unique base keys (Key0 to Key14)
    base_keys = {f"Key{i}" for i in range(15)}
    
    # Assert all base keys exist in key_map
    for bk in base_keys:
        assert bk in key_map
        
    # Assert all base keys map to unique character bindings
    mapped_chars = {key_map[bk] for bk in base_keys}
    assert len(mapped_chars) == 15
    
    # Ensure they map to exactly the classic layout characters
    expected_layout = {'y', 'u', 'i', 'o', 'p', 'h', 'j', 'k', 'l', ';', 'n', 'm', ',', '.', '/'}
    assert mapped_chars == expected_layout

def test_legacy_compatibility_keys():
    """Verify prefix fallback mappings (1Key and 2Key) are preserved in layout map."""
    assert SKY_15_KEY_MAP["1Key0"] == "y"
    assert SKY_15_KEY_MAP["2Key14"] == "/"


def test_mapped_resolver_loads_user32_once(monkeypatch):
    import ctypes
    import sky_music.layouts as layouts
    from sky_music.domain import NoteKey
    from sky_music.layouts import DefaultNoteResolver

    class FakeUser32:
        def MapVirtualKeyW(self, vk, mode):
            return vk + mode + 1

    loads = []

    def fake_windll(name, use_last_error=True):
        loads.append((name, use_last_error))
        return FakeUser32()

    monkeypatch.setattr(layouts, "_USER32", None)
    monkeypatch.setattr(ctypes, "WinDLL", fake_windll)

    resolver = DefaultNoteResolver(SKY_15_KEY_PROFILE)
    assert resolver.resolve_scan_code(NoteKey("Key0"), mode="mapped") != 0
    assert resolver.resolve_scan_code(NoteKey("Key1"), mode="mapped") != 0

    assert loads == [("user32", True)]

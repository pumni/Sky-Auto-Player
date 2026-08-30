# Phase 9 — GUI canonical evidence

Phase 9 makes the packaged Tauri/React desktop application the canonical v4
user-facing surface. The supported Textual fallback remains available through
`play.bat` or `Sky-Auto-Player-Core.exe --tui`; source development continues to
use `uv run python src/main.py`.

## Screenshot provenance

These are real Windows Tauri-window captures already accepted during the
non-physical desktop slice. They were captured against the production
`core_main.py` admission path, not a browser mock. Phase 9 reuses them for the
canonical README/site presentation rather than relabelling the old Textual
capture as the desktop product.

| Surface | Evidence | Dimensions | SHA-256 |
| --- | --- | ---: | --- |
| Library | [`library-real-tauri.png`](../desktop-nonphysical/library-real-tauri.png) | 1214×798 | `64f8a7c13fb5717d66e898a6ed2ca3bab6b24a68c91e8898f5f7a19f3446a8b6` |
| Minimum window | [`minimum-real-tauri.png`](../desktop-nonphysical/minimum-real-tauri.png) | 1214×798 | `c5bbf89402cadb92b900fba468a0cd594bf8aadaaf60cf36d4a4ec837d596db6` |
| Song Detail | [`detail-real-tauri.png`](../desktop-nonphysical/detail-real-tauri.png) | 1214×798 | `454346d2db5eb490f7d0a8630073bb72a9f38a45fd26e58ddbbfd70029956e21` |
| Settings | [`settings-real-tauri.png`](../desktop-nonphysical/settings-real-tauri.png) | 1214×798 | `39525818205db19a2a2622d52fa2bcb58405860ffb75895b4c2ae5753ab54915` |

The source evidence set also contains the real search view. The captures are
visual evidence only; they do not claim physical playback qualification.

## Product guidance

- Packaged users start `Sky-Auto-Player.exe` for the canonical GUI.
- `play.bat` and `Sky-Auto-Player-Core.exe --tui` are supported fallback paths.
- The Phase 8 package qualification used a controlled local release source for
  the exact artifact. Production GitHub/HTTPS download of public v3.5.0 was not
  E2E-qualified before publication and must not be described as such.

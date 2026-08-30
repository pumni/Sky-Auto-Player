# Phase 9 — GUI canonical evidence

Phase 9 makes the packaged Tauri/React desktop application the canonical v4
user-facing surface. The supported Textual fallback remains available through
`play.bat` or `Sky-Auto-Player-Core.exe --tui`; source development continues to
use `uv run python src/main.py`.

## Screenshot provenance

The four PNGs below are real Windows Tauri-window captures from the accepted
non-physical desktop slice. The real local Core used the production
`core_main.py` admission path; there was no browser mock or fake bridge and no
physical playback. The source capture was made at
`capture_repo_head: 9e9d97e61b391d0dad39868d05d15f64763137cb`.

`capture_command: debug Tauri shell with production core_main.py admission;
Windows window capture after the ready UI state`
`capture_context: Windows 11, Wry/Tauri desktop window, source Core, no
browser mock or fake bridge`
`candidate_repo_head: 37eece13111babf01b84f56add4d7c5ecaad31c4`

The candidate changes after the source capture are runtime/release identifier
renames and CI/evidence validation only; no UI rendering code changed. The
managed execution environment used for this closure does not expose a usable
desktop compositor: a real current-head Wry process can be started, but its
window is a 14×14 non-rendered surface and screen/PrintWindow capture is blank.
The source PNGs are therefore retained as historical real-Tauri evidence and
are not claimed to be freshly captured from the candidate head.

| Surface | Evidence | Dimensions | SHA-256 |
| --- | --- | ---: | --- |
| Library | [`library-real-tauri.png`](../desktop-nonphysical/library-real-tauri.png) | 1214×798 | `64f8a7c13fb5717d66e898a6ed2ca3bab6b24a68c91e8898f5f7a19f3446a8b6` |
| Minimum window | [`minimum-real-tauri.png`](../desktop-nonphysical/minimum-real-tauri.png) | 920×620 | `c5bbf89402cadb92b900fba468a0cd594bf8aadaaf60cf36d4a4ec837d596db6` |
| Song Detail | [`detail-real-tauri.png`](../desktop-nonphysical/detail-real-tauri.png) | 1214×798 | `454346d2db5eb490f7d0a8630073bb72a9f38a45fd26e58ddbbfd70029956e21` |
| Settings | [`settings-real-tauri.png`](../desktop-nonphysical/settings-real-tauri.png) | 1214×798 | `39525818205db19a2a2622d52fa2bcb58405860ffb75895b4c2ae5753ab54915` |

The source evidence set also contains the real search view. The captures are
visual evidence only; they do not claim physical playback qualification. A
fresh exact-candidate capture must be produced in an interactive Windows
session before these images can be promoted from historical evidence to
exact-current canonical release evidence.

## Product guidance

- Packaged users start `Sky-Auto-Player.exe` for the canonical GUI.
- `play.bat` and `Sky-Auto-Player-Core.exe --tui` are supported fallback paths.
- The Phase 8 package qualification used a controlled local release source for
  the exact artifact. Production GitHub/HTTPS download of public v3.5.0 was not
  E2E-qualified before publication and must not be described as such.

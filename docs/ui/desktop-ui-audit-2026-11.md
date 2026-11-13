# Desktop UI/UX audit — real Windows captures (2026-11-11)

Review branch: `ui/workbench-clarity-2026-11`. This review was driven by two native Windows screenshots supplied in the conversation, an approximately 1649×927 window and a smaller approximately 987×691 window, plus source review of the Tauri/React desktop. The screenshots themselves are not committed. All uncaptured screens and states remain code-reviewed, **not visually qualified**.

## Primary interaction model

- **Selected** means selected for Inspector; **Playing** means owned by the active playback session. Their visual and accessible states must not be conflated.
- **Auto Play** continues with the next song *only after the current song finishes successfully*. It does not loop or repeat a track and must not use the Repeat icon.
- **Test playback** is dry-run playback and must not produce physical keyboard input. Normal Play remains a separate action with existing admission safeguards.
- Native warnings, pitch assumptions and conversion approval are safety-relevant content and must not be hidden merely to reclaim vertical space.

## Audit by surface

| Priority | Surface | Observation and impact | Result / follow-up |
| --- | --- | --- | --- |
| P0 | Dock: Auto Play | The previous ListMusic icon communicated only a list, and a native `title` did not give consistently visible hover feedback in the Windows capture | **Changed:** Lucide ListVideo, conveying queued playback, plus React Aria Tooltip on hover/focus with On/Off and next-song explanation. Verify in native WebView2. |
| P1 | Playback profile | Wide two-sided field rows and cramped Timing Margin made the popover unnecessarily dense; advanced values and Test competed for attention | **Changed:** three aligned primary selects, separate full-width margin stepper, secondary calculated-timing disclosure, separated preview group |
| P1 | Disabled preview | Test was disabled with no visible reason when no song was selected | **Changed:** explanatory no-song or busy-session copy, without bypassing playback admission checks |
| P1 | Player icon comprehension | Auto, Profile and Inspector are icon-only controls; names need to be perceivable through focus and hover | **Changed:** Auto and Inspector explicit tooltips; Profile retains an accessible name and title. **Later:** standardize the tooltip pattern across all icon-only controls |
| P1 | Status badges across desktop | Player Dock risk, library metadata warnings and Test used different sizing, colored fills or text treatment; colors competed with theme | **Changed:** shared StatusBadge with theme tokens for text, borders, surface and radius. Soft neutral labels use one visual language; medium/high risk and metadata warnings use only a small semantic warning/danger dot plus explicit text, while Test is plain/quiet. Keep native confirmation and risk rationale unchanged. Verify Graphite/other dark palettes, Light, forced colors and narrow Windows WebView2. |
| P1 | Player Dock: playback mode clarity | LIVE/PHYSICAL beside Paused introduced redundant warning color, and a 48px cap cut text mid-word | **Changed:** remove the default-mode badge. Dry run uses shared StatusBadge in quiet appearance (compact Test at narrow widths); status accessibility and tooltip retain explicit no-input messaging. |
| P2 | Player Dock: progress caps | The native `<progress>` fill appeared square at the leading edge near 0:02, despite border radius on its inner value | **Changed:** remove the 1px border from the 6px track and clip the fill to the rounded track. Keep native, read-only progress semantics; verify at 0/2 seconds, mid-track and completion on WebView2. |
| P1 | Browser and Inspector identity | Selected and playing identities are independent; both the browser header and Player Dock were poor homes for list navigation | **Changed:** Locate selected moves to the Built-in Songs pager footer beside the row count, using the neutral Lucide LocateFixed icon. Enabled for a valid selected row; centers it or returns to its built-in page on demand without automatic navigation. Verify native focus and compact footer. |
| P0 | Playback confirmation | Screenshot showed a full-width, three-column confirmation with a huge empty Cancel card and raw timing analytics occupying the library | **Changed:** centered, width-constrained confirmation; compact decision buttons, Not now cancellation and per-option expandable estimates. Risk headline and full reason remain visible. |
| P1 | Compact Timing Margin | A single margin setting occupied a full card within the Playback profile | **Changed:** condensed into a single line with small recommendation and +/- beside the value, preserving backend increments. |
| P1 | Narrow Inspector | Pitch warning and compatibility content occupy most of the visible details pane; downstream risk guidance requires scrolling | **Open:** verify that warnings and necessary confirmation controls are discoverable and fully keyboard-reachable at 800×560; never suppress safety content |
| P2 | Track browser sticky header | At the screenshot scroll position, the first partially visible track row sits immediately under the sticky header and looks clipped | **Open:** visually qualify scroll layering / top-edge clipping before changing virtualized row positioning |
| P2 | Pager footer | Footer previously took more vertical space than required | **Changed earlier:** 32px targets, compact container and no extra bottom padding; preserve page-jump focus/geometry |
| P2 | Empty Inspector | Tiny top-left placeholder over a large panel did not establish context | **Changed earlier:** centered no-selection cue |
| P2 | Library and Chrome | Collapsed navigation works but its playlist monograms need hover context; titlebar refresh/settings and additional icon menu actions were missing consistent fallback labels | **Partial:** navigation already uses accessible labels and a playlist tooltip; added missing native title labels in key toolbars and menus. Verify Tauri behavior |
| P2 | Search / pagination | Search is global to the selected source, while Built-in pagination resets/repositions the virtual table | **Open QA:** verify query, page switch, keyboard selection and reset without unexpected focus loss |
| P2 | Diagnostics / Runtime | Dry runs are not physical sender samples; older UI showed misleading unmeasured metrics | **Changed earlier:** explicit unmeasured state; test real and dry-run export behavior separately |
| P2 | Modal and auxiliary screens | Settings, updates, import, calibration, donation, diagnostics detail are not present in the supplied captures | **Unverified visually:** perform a dedicated walkthrough, including high-contrast, errors, loading/empty and keyboard states |

## Lucide icon language

| Action | Icon | Rationale |
| --- | --- | --- |
| Built-in Songs | `Disc3` | recognizable music catalog / bundled songs |
| Playlist | `ListMusic` | music collection |
| Auto Play next song | `ListVideo` | list with Play glyph; not Repeat or Shuffle |
| Reload library | `RefreshCw` | refresh/reload |
| Playback profile | `SlidersHorizontal` | timing adjustments |
| Safe dry-run test | `FlaskConical` | testing rather than physical output |
| Inspector toggle | `PanelRight` | right panel |
| Current/playing track | `Music2` | identifies playback in a non-audio app |
| Liked Songs | `Heart` | favorite |
| Playback | `Shuffle`, `SkipBack`, `Play`, `Pause`, `SkipForward`, `Square` | standard transport semantics |
| Row / playlist actions | `MoreHorizontal`, `ListPlus`, `Pencil`, `Trash2` | actions preserve labels and tooltips |

Icon meaning should be independently accessible via `aria-label` (or visible text), not color alone. State toggles additionally use `aria-pressed`, and their tooltips explain whether the setting is On or Off.

## Required acceptance checks

1. At 1649×927, 1005×615, 987×691 and 800×560, verify no dock-control overlap and no horizontal overflow. At very small heights ensure the profile popover scrolls and remains in the viewport.
2. Hover and keyboard-focus **Auto Play**: a visible tooltip describes the next-song behavior, announces current On/Off, and does not collide with the dock. Toggle the button and confirm Settings reflects the persisted value.
3. Open **Playback profile**: Hold / Tempo / FPS are aligned and remain editable, Timing Margin is full-width, the recommendation is readable, and the calculated values are hidden until expanded.
4. In the no-song state, Test is disabled with an actionable reason. With a selected song and idle transport, Test uses `dry_run: true` and sends no physical input.
5. Inspect a selected song while the table is scrolled elsewhere; verify Inspector retains identity without unexpected table auto-scroll. Record discoverability issues for the proposed Locate action.
6. Test the inspector pitch-risk warning, the conversion confirmation, pagination (including numeric jump), runtime diagnostics and recovery for errors.
7. Run keyboard-only navigation, screen-reader accessible names, palette/Light/Dark, Windows High Contrast and reduced-motion checks.
8. Run from the desktop app: `bun run check` and `bun run test:e2e`. Run from the root: `cargo xtask check all`. Inspect the native Tauri shell after building `native_calibration.exe` on a new worktree.
9. On native WebView2 at 1649×927, 1005×615, and 800×560: inspect the timeline at 0:00, 0:02, midpoint and completion for rounded caps; verify normal playback has no redundant mode badge, dry run retains neutral Test / Test · no keys with a complete no-input explanation, and the pager-footer LocateFixed action stays within the compact row beside the song count, centers the selected built-in row, and never changes selection.

- The Donate heading uses Lucide HandHeart to communicate support; Liked Songs keeps its Heart icon.

**Validation status:** code changes and tests have been committed to the review branch; Bun/Playwright, the Rust/native gates, and native screenshot comparison have **not** been executed by the assistant. Do not merge on source inspection alone.

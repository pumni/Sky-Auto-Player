# Workbench clarity review — Windows QA

Review branch: `ui/workbench-clarity-2026-11`. This review changes the frontend presentation and dry-run discoverability, not the native sender implementation.

## Install and verify

Use a separate Git worktree. From the main repository root:

```powershell
git fetch origin ui/workbench-clarity-2026-11
git worktree add --track -b ui-workbench-review ../Sky-Auto-Player-UI origin/ui/workbench-clarity-2026-11
cd ../Sky-Auto-Player-UI/apps/desktop
bun install --frozen-lockfile
bun run typecheck
bun run lint
bun run format:check
bun run test
bun run test:e2e
# From the worktree root, build the Tauri resource before the first native dev run:
cd ../..
cargo xtask calibration build
cd apps/desktop
bun run tauri:dev
```

Run the canonical `cargo xtask check all` from the new worktree root before proposing a merge, as required by CONTRIBUTING.md.

## Manual acceptance

- Locate selected sits beside the row-count text in the Built-in Songs pager footer, not in Player Dock or browser header. The neutral Lucide LocateFixed (crosshair) icon has a Go to selected song tooltip, is disabled until a valid song is selected, and centers that row (or returns to its page) only when clicked. It does not change selection. The footer stays compact at narrow widths.
- At 1366×768 and 800×560, distinguish the playing row (accent title and rail) from the selected row (neutral rail and selected background), including their combined state.
- At 1649×927, 1005×615 and 800×560, the player dock is icon-only on the right: no standalone Test button, Auto label or long numeric profile string. Its controls must not overlap or clip.
- Playback profile is a labeled, scrollable popover with three aligned Hold/Tempo/FPS selectors, a full-width Timing Margin stepper and readable recommendation, and a collapsed Calculated timing section. Its Test playback (no input) action remains available and must never send physical keystrokes.
- Hover and keyboard-focus the Auto Play button. The explicit tooltip must describe On/Off and next-song continuation; changing Auto Play must update the Settings switch. Inspect the same tooltip treatment on the Inspector toggle.
- Built-in pager has a compact footer (about 37px including border and padding), 32px click targets, stable inline page-jump editing, and no extra bottom padding.
- Badge design is shared across the Player Dock risk label, dry-run Test · no keys, and Track Browser metadata states: neutral theme surfaces/borders/text and a subtle warning/danger dot where applicable. No saturated filled warning/danger pills. Verify dark palettes, Light, and forced-colors; retain explicit level text, safety explanations, and non-dismissable playback admission prompts.
- Normal playback shows no redundant mode badge; the current playback state remains the primary status. Dry run shows quiet inline Test · no keys (compact Test at 1100px or less), not a colorful pill. The status and tooltip still explain that no physical input is sent; preserve severity information separately. Verify at 800×560.
- Verify the native progress bar is a visually centered, Spotify-like 4px track (inside its unchanged 6px layout slot) with rounded leading/trailing caps at 0:00, 0:02, mid-track and completion. It remains a read-only `<progress>`, not a seek slider.
- Check the native Tauri/Windows app; Chromium mock screenshots alone cannot verify keyboard input or focus handling. With no song selected, Inspector should show a centered action cue instead of a tiny top-left line.
- Lucide icon semantics: Disc3 for the bundled music catalog, Heart for likes, ListMusic for playlists/auto-queue, RefreshCw for reload, LocateFixed for Locate selected beside the pager range, SlidersHorizontal for profile, PanelRight for Inspector, FlaskConical for a safe test, Music2 for current-track identity, and the usual transport shapes for playback. All icon-only controls need accessible names and tooltips.
- In Song Details, loss reasons and conversion consent remain visible. Technical counters expand independently, and low-risk explanations collapse.
- Play on a ready/low-risk song should show only the inline Preparing/Starting status: no transient Prepared timing advisory popover may flash between Prepare and Start. The ready-plan advisory data remains in the prepared result. A real risk confirmation must still appear and remain actionable.
- A risk confirmation no longer stretches across the library: visible risk text, compact option buttons, collapsible per-option timing estimates, and a small Not now action that cancels prepared playback without starting. Check both one-decision (native screenshot) and three-decision (mock) variants; opening estimates must not clip at 800×560.
- In Playback profile, Timing Margin is a compact single-line setting (label/recommendation plus increment controls) rather than a large separate card; its min/max disable behavior and persisted backend values must remain correct.
- In Runtime → Performance, dry run shows a single unmeasured-state message; real samples remain visible for physical sessions.
- Verify Donate heading uses HandHeart (not Heart); Liked Songs still uses Heart. Inspect Light mode, forced colors, focus rings, reduced-motion behavior, and scrollbars.
- Validate sender-metadata export with and without a real session. Export remains backend-owned because frontend session identity alone does not establish trace availability.

## Workbench resize lane

- Pane borders, rounded corners and the **4px visual gutter** remain unchanged. The separator occupies all 4px of the existing gutter, while the transparent pointer-acquisition zone extends slightly beyond it; hover/focus/drag highlights the full 4px lane with a clearly visible accent and rounded caps across the full height, with no vertical inset.
- Drag the Navigator and docked Inspector dividers at 1366×768 and 800×560; verify there are no dead pixels in the gutter, no surprising pane jump, unchanged keyboard arrows / Home / End / Enter reset, and that one final persisted width survives reload.
- The layout policy still updates live widths on animation frames without persisting each pointer movement; review performance on native WebView2 rather than treating simulated event counts as measured FPS.

## Full UI audit

The prioritized review and remaining tasks are recorded in [Desktop UI/UX audit — real Windows captures](desktop-ui-audit-2026-11.md). The screenshots only show Library, Player Dock and Inspector; Settings, Update, Calibration, Import, Runtime recovery and other dialogs still require a separate native visual pass.

## Screenshot evidence

After native review, run `bun run ui:refresh` in `apps/desktop` if refreshed Chromium screenshots are needed. Review `docs/ui/current` and commit regenerated files separately; screenshots are not a substitute for native input verification.

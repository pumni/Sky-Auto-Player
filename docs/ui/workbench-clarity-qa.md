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

- When an inspected built-in song is outside the current scroll range or on another page, Locate selected appears and returns to it on demand. It should not appear for a visible song and should not auto-scroll the table.
- At 1366×768 and 800×560, distinguish the playing row (accent title and rail) from the selected row (neutral rail and selected background), including their combined state.
- At 1649×927, 1005×615 and 800×560, the player dock is icon-only on the right: no standalone Test button, Auto label or long numeric profile string. Its controls must not overlap or clip.
- Playback profile is a labeled, scrollable popover with three aligned Hold/Tempo/FPS selectors, a full-width Timing Margin stepper and readable recommendation, and a collapsed Calculated timing section. Its Test playback (no input) action remains available and must never send physical keystrokes.
- Hover and keyboard-focus the Auto Play button. The explicit tooltip must describe On/Off and next-song continuation; changing Auto Play must update the Settings switch. Inspect the same tooltip treatment on the Inspector toggle.
- Built-in pager has a compact footer (about 37px including border and padding), 32px click targets, stable inline page-jump editing, and no extra bottom padding.
- Test playback shows a TEST badge and sends no physical keyboard input. Normal playback displays a LIVE badge without falsely claiming that paused or nonadmitted sessions are sending keys.
- Check the native Tauri/Windows app; Chromium mock screenshots alone cannot verify keyboard input or focus handling. With no song selected, Inspector should show a centered action cue instead of a tiny top-left line.
- Lucide icon semantics: BookMusic for built-in sheets, Heart for likes, ListMusic for playlists/auto-queue, RefreshCw for reload, SlidersHorizontal for profile, PanelRight for Inspector, FlaskConical for a safe test, Music2 for current-track identity, and the usual transport shapes for playback. All icon-only controls need accessible names and tooltips.
- In Song Details, loss reasons and conversion consent remain visible. Technical counters expand independently, and low-risk explanations collapse.
- A risk confirmation no longer stretches across the library: visible risk text, compact option buttons, collapsible per-option timing estimates, and a small Not now action that cancels prepared playback without starting. Check both one-decision (native screenshot) and three-decision (mock) variants; opening estimates must not clip at 800×560.
- In Playback profile, Timing Margin is a compact single-line setting (label/recommendation plus increment controls) rather than a large separate card; its min/max disable behavior and persisted backend values must remain correct.
- In Runtime → Performance, dry run shows a single unmeasured-state message; real samples remain visible for physical sessions.
- Inspect Light mode, forced colors, focus rings, reduced-motion behavior, and scrollbars.
- Validate sender-metadata export with and without a real session. Export remains backend-owned because frontend session identity alone does not establish trace availability.

## Full UI audit

The prioritized review and remaining tasks are recorded in [Desktop UI/UX audit — real Windows captures](desktop-ui-audit-2026-11.md). The screenshots only show Library, Player Dock and Inspector; Settings, Update, Calibration, Import, Runtime recovery and other dialogs still require a separate native visual pass.

## Screenshot evidence

After native review, run `bun run ui:refresh` in `apps/desktop` if refreshed Chromium screenshots are needed. Review `docs/ui/current` and commit regenerated files separately; screenshots are not a substitute for native input verification.

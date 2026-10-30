# Desktop Workbench

The app shell presents the Library Navigator, Track Browser, Inspector, and fixed Player Dock.
`desktop/src/app/App.tsx` renders `<Workbench />` and `<PlayerBar />` as siblings inside
`.app-shell`. `Workbench.tsx` contains the Library Navigator, Track Browser, Inspector, and their
separators only. `PlayerBar.tsx` owns the fixed 92 px Player Dock outside the Workbench subtree.
The Inspector presents Song Details and Runtime Diagnostics in both docked and compact layouts.

## Frontend ownership

`desktop/src/app/App.tsx` owns the app-shell composition: it renders `AppTitleBar`, `<Workbench />`,
and `<PlayerBar />` as sibling children inside `.app-shell`.
`desktop/src/components/workbench/Workbench.tsx` composes the Library Navigator, Track Browser,
Inspector, and their separators only. `desktop/src/components/player/PlayerBar.tsx` owns the fixed
92 px Player Dock and is outside the Workbench subtree.
`desktop/src/components/workbench/workbenchGeometry.ts` is the pure geometry owner;
`MIN_TRACK_BROWSER_WIDTH` is the single 420 px source passed to the workbench as
`--min-track-browser-width`. `desktop/src/components/workbench/useWorkbenchLayout.ts` connects
measured viewport changes and separator input to the geometry policy.
`desktop/src/components/workbench/workbenchDragPolicy.ts` keeps resize rebasing and in-session
topology decisions transient. `desktop/src/components/workbench/useWorkbenchPreferences.ts`
loads and updates the UI layout, while `desktop/src/state/workbenchPreferences.ts` validates and
persists the existing v5 preference contract.

Pointer moves update live geometry without writing preferences. Pointer release and explicit
keyboard resize commits persist the final bounded widths. Viewport-driven collapse, overlay
presentation, and temporary widths do not become saved preferences.

`desktop/src/components/settings/SettingsPanel.tsx` retains Settings modal, category navigation,
and focus ownership; `desktop/src/components/settings/SettingsCategoryContent.tsx` renders the
selected category and palette selector. `desktop/src/components/utility/DiagnosticsView.tsx`
composes Diagnostics, with `desktop/src/components/utility/diagnosticsPresentation.ts` and the
focused playback, readout, event, timing, and performance components owning presentation of their
respective data. These UI splits do not change the canonical Zustand store, typed bridge, native
settings schema, or native diagnostics authority.

## Inspector presentation

The Inspector remains docked when the measured workbench width can retain the Navigator rail
(56 px for Classic, 48 px for Graphite), the 420 px minimum Track Browser, standard workbench
gutters, and at least 280 px of Inspector content. The existing geometry solver determines that
capacity. When those constraints do not fit, the Inspector becomes a temporary right-side overlay
over the workbench content. Its width is computed from the measured
CSS workbench width and available space, and is not a persisted pane preference. Classic caps the
responsive overlay at 400 px. Graphite targets 45% of the measured width, bounded to 288–352 px,
then caps that target to the available width; at very narrow widths the available-width cap can
make the actual overlay narrower than 288 px. The overlay stays inside the workbench row, above the
Track Browser, and clear of the Player Dock.

## Desktop minimum and responsive fallback

The Tauri main window minimum is configured as 800×560 logical pixels. The Inspector is docked at
that minimum. The overlay below 764 CSS pixels remains a responsive fallback; the current native
minimum prevents reaching that width by ordinary desktop resizing. E2E cases below 800×560 use
Playwright's Chromium viewport emulation and verify responsive behavior only, not a supported native
window size or live Windows geometry.

At 125% Windows display scaling, 800 logical pixels map to about 1000 physical pixels. A Playwright
`deviceScaleFactor` of 1.25 only emulates browser device scale and does not model the native window
minimum or establish live DPI behavior.

The overlay is non-modal. It is a labelled `region`, does not dim the application, does not trap
focus, and does not cover the title bar or Player Dock. The user can continue to use visible Track
Browser rows underneath or beside it. Selecting a row updates the open Song Details view and leaves
the Inspector open. Pointer interaction outside the Inspector does not dismiss it automatically.

The Player Dock utility toggle, the Inspector Close button, and Escape close the Inspector. Escape
works while focus is inside the Inspector or elsewhere in the application, unless another focused
control has already handled that key. Opening the compact overlay from the Player Dock moves focus
to the selected Inspector tab. Automatic dock/overlay transitions caused by window resizing do not
move focus. Close and Escape restore focus to the utility toggle when it is still connected. There
is no focus trap.

## Preferences and scrolling

The saved docked Inspector width remains in the existing v5 preference range of 280–440 px. The
Navigator expanded width and the Inspector open/closed preference are also persisted as before.
Responsive presentation and temporary geometry are computed from measured CSS width; resizing or
changing DPI does not write those preferences. Docked separators remain available to pointer and
keyboard users; the overlay has no resize separator.

The geometry contract keeps the Track Browser at least 420 px, track rows at 46 px, and the Player
Dock at 92 px. The active Workbench layout and CSS variable values are measured by unit and browser
E2E tests; pane geometry must remain within the viewport, including when a separator reaches its
clamped bounds.

Song Details and Runtime Diagnostics retain their own scrollable content. Runtime metrics, timing
plots, events, and trace export remain available in either presentation. The Inspector ends above
the Player Dock, which keeps its own independent geometry and controls.

## Palette and semantic color roles

Graphite remains the default palette with persisted ID `graphite`; `data-color-mode="dark"` remains
unchanged. Settings calls it **Graphite (Default)**. The five other persisted palette IDs remain
selectable and their palette-specific colors stay in `desktop/src/styles/themes.css`.

Production Graphite is the existing A palette: accent ink and focus `#B2A5E9`, primary fill
`#705EAE`, hover `#7564B4`, pressed `#6553A0`, selected surface `#25242C`, search focus `#9688C6`,
and active toggle ink `#B68CFF`. Semantic roles remain separate: accent text/ink, primary action
fill and on-fill text, focus indication, search focus, selected surfaces/indicators, toggle state,
and danger/warning/success. Compatibility `--workbench-*` aliases resolve through those roles.

Soft Graphite B is a review preview only. Its override exists in the opt-in
`desktop/tests/e2e/graphite-package-c.spec.ts` A/B capture and
`docs/evidence/issue-563-package-c/`; it is not a production selector or shipped
`desktop/src/styles/themes.css` palette. Graphite A stays selected unless the owner and Agent Lead
explicitly approve a future visual decision.

The automated contrast gates use 4.5:1 for text and 3:1 for essential non-text/focus indicators.
They compute CSS token/color-pair ratios and assert rendered interaction styles; they do not
sample every screenshot pixel. Forced-colors and reduced-motion browser tests use Chromium media
emulation and do not prove Windows High Contrast or packaged WebView2 behavior.

## Regression and evidence contract

- `desktop/src/state/store.test.ts` covers latest-intent request fencing, catalog generation changes, bounded
  page caching, Like/membership invalidation, selected-song identity, full-source playback, and
  session retirement races. `desktop/tests/e2e/library.spec.ts` covers 100-result page boundaries,
  page 101 navigation, virtualization, and playback across pages.
- `desktop/tests/e2e/playback.spec.ts` and
  `desktop/tests/e2e/playback-recovery.spec.ts` cover transport geometry, start and retirement
  races, recovery, and dry-run versus physical labels.
- `desktop/src/components/workbench` unit tests plus `desktop/tests/e2e/workbench-layout.spec.ts`,
  `desktop/tests/e2e/workbench-resize.spec.ts`, and
  `desktop/tests/e2e/workbench-state-regression.spec.ts` cover pane geometry, pointer/keyboard
  resize, focus/ARIA state, responsive topology, and preference v5 persistence.
- `desktop/tests/e2e/settings-diagnostics.spec.ts`,
  `desktop/tests/e2e/palette-appearance.spec.ts`,
  `desktop/tests/e2e/graphite-final-ui-polish.spec.ts`, and
  `desktop/tests/e2e/graphite-package-c.spec.ts` cover modal/accessibility, Diagnostics
  truthfulness, all six palettes, focus and semantic color roles, forced-colors, and reduced-motion
  behavior.

Browser tests use the mock bridge. The 640×448 overlay cases are responsive browser fallbacks,
not supported native sizes. Packaged WebView2 resize/startup measurements and their baseline
comparison, when available, are recorded separately in
[`issue-563-package-d evidence`](evidence/issue-563-package-d/README.md); neither browser mocks nor
packaged dry runs send gameplay input or establish in-game behavior.

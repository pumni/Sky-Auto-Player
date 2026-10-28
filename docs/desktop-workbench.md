# Desktop Workbench

The desktop workbench keeps the Library Navigator, Track Browser, right Inspector, and fixed
Player Dock as separate regions. The Inspector presents Song Details and Runtime Diagnostics in
both docked and compact layouts.

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

Song Details and Runtime Diagnostics retain their own scrollable content. Runtime metrics, timing
plots, events, and trace export remain available in either presentation. The Inspector ends above
the Player Dock, which keeps its own independent geometry and controls.

# Issue #565 — Package B rendered evidence

- **Capture source SHA:** `b5b6f68a3f7257225e61be1e13d7981f7d077f2b`
- **Capture:** 2026-10-30; see [`capture-manifest.json`](capture-manifest.json) for the exact timestamp and runtime metadata.
- **Browser environment:** Windows 11 Home, build `10.0.26300`, x64; Playwright Chromium `156.0.8078.4`, device scale factor 1. The app ran through its deterministic browser mock bridge with `prefers-color-scheme` emulation. It was not a packaged Tauri/WebView2 run and sent no game input.

The capture contains 22 full-page PNGs: Light and Graphite Dark at all five requested CSS-pixel viewports, plus Appearance Settings, populated Runtime Diagnostics, playback error, confirmation, focus/hover, and the browser-only compact utility overlay. The 800×560 view is the native minimum-size layout check. The 640×448 view is a browser responsive fallback, not a supported native window size. `Blue Bird` is selected while `Candle Run` is playing in populated scenes.

## Light / Graphite Dark screenshot pairs

| State                                                               | Light                                            | Graphite Dark                                            |
| ------------------------------------------------------------------- | ------------------------------------------------ | -------------------------------------------------------- |
| Populated 1366×768                                                  | [PNG](light/populated-1366x768.png)              | [PNG](graphite-dark/populated-1366x768.png)              |
| Populated 1200×760                                                  | [PNG](light/populated-1200x760.png)              | [PNG](graphite-dark/populated-1200x760.png)              |
| Populated 1024×768                                                  | [PNG](light/populated-1024x768.png)              | [PNG](graphite-dark/populated-1024x768.png)              |
| Populated 800×560                                                   | [PNG](light/populated-800x560.png)               | [PNG](graphite-dark/populated-800x560.png)               |
| Populated 640×448                                                   | [PNG](light/populated-640x448.png)               | [PNG](graphite-dark/populated-640x448.png)               |
| Settings · Appearance                                               | [PNG](light/settings-appearance-1366x768.png)    | [PNG](graphite-dark/settings-appearance-1366x768.png)    |
| Diagnostics · Runtime                                               | [PNG](light/diagnostics-runtime-1366x768.png)    | [PNG](graphite-dark/diagnostics-runtime-1366x768.png)    |
| Playback error                                                      | [PNG](light/playback-error-1366x768.png)         | [PNG](graphite-dark/playback-error-1366x768.png)         |
| Playback confirmation                                               | [PNG](light/playback-confirmation-1366x768.png)  | [PNG](graphite-dark/playback-confirmation-1366x768.png)  |
| Search focus + Like hover                                           | [PNG](light/focus-hover-1366x768.png)            | [PNG](graphite-dark/focus-hover-1366x768.png)            |
| Compact utility overlay 640×448 — Diagnostics Runtime loading state | [PNG](light/compact-utility-overlay-640x448.png) | [PNG](graphite-dark/compact-utility-overlay-640x448.png) |

The compact overlay captures show the Diagnostics Runtime tab while it displays `Loading diagnostics…`.
Treat those two PNGs as **loading-state examples**, not settled Diagnostics runtime evidence. The
1366×768 Diagnostics Runtime screenshots above show the settled dry-run panel.

## Computed contrast

[`computed-contrast.json`](computed-contrast.json) records text samples gathered from each rendered page with `getComputedStyle`, including ancestor opacity and composited backgrounds. Each mode has 11 scenes and 979 visible text/placeholder samples. The minimum normal-text ratio was **4.97:1 in Light** and **4.93:1 in Graphite Dark**; every captured scene has zero text failures. Screenshot scenes contain zero inactive-control text exemptions.

The rendered-state E2E assertions also calculate contrast from browser-computed colors for important controls and indicators:

| Light state / role                                            | Computed colors                                                     |                    Ratio |
| ------------------------------------------------------------- | ------------------------------------------------------------------- | -----------------------: |
| Search focus ring on canvas                                   | `#0B58A5` / `#FFFFFF`                                               |                   7.11:1 |
| Selected-row indicator on selected surface                    | `#0B58A5` / `#EAF2FD`                                               |                   6.30:1 |
| Meaningful control edge on canvas / shell                     | `#767F8A` / `#FFFFFF`, `#F7F8FA`                                    |          4.06:1 / 3.82:1 |
| Playing-row indicator on shell                                | `#767F8A` / `#F7F8FA`                                               |                   3.82:1 |
| Scrollbar thumb on canvas / shell                             | `#808B97` / `#FFFFFF`, `#F7F8FA`                                    |          3.47:1 / 3.26:1 |
| White action label on primary default / hover / pressed fills | `#FFFFFF` / `#0B63C5`, `#0958AE`, `#084C98`                         | 5.82:1 / 6.96:1 / 8.41:1 |
| Status text on success / warning / danger tints               | `#1F7A47` / `#E9F5ED`; `#8D5A08` / `#FFF5DF`; `#B4232F` / `#FCEBED` | 4.77:1 / 5.39:1 / 5.66:1 |

The hairline separator `#DFE4EB` is decorative. Disabled controls are covered by the browser interaction test; WCAG 2.2 SC 1.4.3 does not require inactive-control text contrast. No such text exemption was needed by the captured scenes. Focus, selected/playing separation, hover, pressed, forced-colors, and reduced-motion states are tested separately in [`light-appearance.spec.ts`](../../../desktop/tests/e2e/light-appearance.spec.ts) and [`palette-appearance.spec.ts`](../../../desktop/tests/e2e/palette-appearance.spec.ts).

Graphite Dark remains the approved palette. Its existing selected-surface (`#25242C` against `#191A1B`, 1.13:1) and playing divider (`#2A2B2C` against `#191A1B`, 1.23:1) are retained to satisfy the no-dark-change contract. Those low-contrast fills/lines are subdued reinforcement rather than the sole state signal: rows expose `aria-selected`/`aria-current`, playing uses the speaker marker and high-contrast row text, and playback state is also shown in the Player Dock. Graphite Dark visible text and focus/color roles pass their existing palette contrast gates.

## Verification limits

- Chromium `prefers-color-scheme`, forced-colors, and reduced-motion emulation do not qualify Windows High Contrast or packaged WebView2 behavior.
- The browser mock does not exercise native settings migration, Tauri window reveal, real Windows theme-change events, DPI scaling, or in-game behavior. Package A migration remains unchanged: fresh config follows Windows; legacy configs remain Dark and preserve their chosen dark palette.
- The native window background remains fixed at RGB `(18, 19, 20)` and may show before WebView content paints. The known 2.5-second reveal fallback is reserved for Package C packaged-Windows qualification.

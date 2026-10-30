# Issue #563 Package C: Graphite A/B evidence

## Decision boundary

**B / Soft Graphite is a review preview only.** Production continues to use Graphite A. No palette ID, persisted value, or shipped color has been changed pending Agent Lead and owner visual approval.

The Settings label is **Graphite (Default)**. Its persisted palette ID remains `graphite`, and the document remains in dark mode.

## Capture provenance

- Rendered source SHA: `c074b88660891567252c17e3153e40193f69d9ea`
- Environment: Windows 11 Home (`10.0.26300`), x64
- Browser: Playwright Chromium `156.0.8078.4`, device scale factor 1
- Captures use CSS-pixel browser viewports and the deterministic mock bridge. They do not establish Tauri/native window behavior.
- The 640×448 inspector capture is the issue's Chromium responsive fallback; it is not a supported native minimum-size claim.
- [Full computed render, media, geometry, token, contrast, and interaction measurements](render-measurements.json)

## Matched screenshots

| Scene | Graphite A | Soft Graphite B preview |
| --- | --- | --- |
| Populated, selected Blue Bird / playing Candle Run, 1366×768 | [PNG](A-current/populated-1366x768.png) | [PNG](B-preview/populated-1366x768.png) |
| Compact, 800×560 | [PNG](A-current/compact-800x560.png) | [PNG](B-preview/compact-800x560.png) |
| Inspector overlay fallback, 640×448 | [PNG](A-current/inspector-overlay-640x448-browser-fallback.png) | [PNG](B-preview/inspector-overlay-640x448-browser-fallback.png) |
| Settings / Appearance | [PNG](A-current/settings-1366x768.png) | [PNG](B-preview/settings-1366x768.png) |
| Playback confirmation | [PNG](A-current/playback-confirmation-1366x768.png) | [PNG](B-preview/playback-confirmation-1366x768.png) |
| Playback error, 800×560 | [PNG](A-current/playback-error-800x560.png) | [PNG](B-preview/playback-error-800x560.png) |
| Diagnostics / Runtime, 1366×768 | [PNG](A-current/diagnostics-1366x768.png) | [PNG](B-preview/diagnostics-1366x768.png) |
| Populated, 1920×1080 | [PNG](A-current/populated-1920x1080.png) | [PNG](B-preview/populated-1920x1080.png) |
| Diagnostics / Runtime, 1920×1080 | [PNG](A-current/diagnostics-1920x1080.png) | [PNG](B-preview/diagnostics-1920x1080.png) |
| Browser forced-colors emulation | [PNG](A-current/forced-colors-1366x768.png) | [PNG](B-preview/forced-colors-1366x768.png) |
| Browser reduced-motion emulation | [PNG](A-current/reduced-motion-1366x768.png) | [PNG](B-preview/reduced-motion-1366x768.png) |

The populated pair preserves a distinct selected row (`Blue Bird`, `aria-selected=true`) and playing row (`Candle Run`, `aria-current=true`) in a dry run. The error pair displays the target-not-found recovery banner. The forced-colors and reduced-motion modes are recorded per scene in the JSON.

### Interactive control crops

| State | Graphite A | Soft Graphite B preview |
| --- | --- | --- |
| Primary action hover | [PNG](A-current/roles/primary-hover-1366x768.png) | [PNG](B-preview/roles/primary-hover-1366x768.png) |
| Primary action pressed | [PNG](A-current/roles/primary-pressed-1366x768.png) | [PNG](B-preview/roles/primary-pressed-1366x768.png) |
| Keyboard-visible focus | [PNG](A-current/roles/primary-focus-1366x768.png) | [PNG](B-preview/roles/primary-focus-1366x768.png) |
| Window close / danger hover | [PNG](A-current/roles/danger-hover-1366x768.png) | [PNG](B-preview/roles/danger-hover-1366x768.png) |

## Token comparison

| Semantic role | Graphite A current | Soft Graphite B preview |
| --- | --- | --- |
| Accent ink | `#B2A5E9` | `#ABA6DF` |
| Primary fill | `#705EAE` | `#6B5C9E` |
| Primary fill hover | `#7564B4` | `#7566A9` |
| Primary fill pressed | `#6553A0` | `#5F528E` |
| Focus ring | `#B2A5E9` | `#ABA6DF` |

Search focus remains its separate `#9688C6` role, active toggle ink remains `#B68CFF`, and the selected surface remains `#25242C` in both variants. B is injected only by the E2E preview test; the production palette CSS contains no B color literals.

## Render and geometry results

Every captured scene reports `layoutFits=true`, no horizontal document overflow, a 46px track row, and a 92px player dock in both variants. The paired workbench boxes are 1366×636 at 1366×768, 800×428 at 800×560, 640×316 at the 640×448 browser fallback, and 1920×948 at 1920×1080. The selected/playing distinction and `graphite`/`dark` attributes also match in both variants.

Typography measurements are 12px for player status and Diagnostics metric values, and 13px for the primary player action. The control focus crop shows a 2px outline; Playwright verifies `:focus-visible` before capture.

## Contrast results

Ratios below are computed from the rendered CSS token values in the evidence run. Thresholds are 4.5:1 for text and 3:1 for focus/boundary indicators.

| Pair | Minimum | Graphite A | Soft Graphite B preview |
| --- | ---: | ---: | ---: |
| Primary text on rest / hover / pressed fills | 4.5:1 | 5.38 / 4.95 / 6.37 | 5.75 / 4.96 / 6.81 |
| Focus ring on panel / raised surface | 3:1 | 7.84 / 6.91 | 7.68 / 6.76 |
| Search focus ring on search surface | 3:1 | 5.09 | 5.09 |
| Accent ink on selected surface | 3:1 | 6.91 | 6.77 |
| Neutral control boundary on canvas | 3:1 | 4.12 | 4.12 |
| Danger button text on rest / hover / pressed fills | 4.5:1 | 5.61 / 4.77 / 7.83 | 5.61 / 4.77 / 7.83 |
| Warning / success text on their surfaces | 4.5:1 | 8.33 / 7.50 | 8.33 / 7.50 |

These are computed color-pair results, not pixel-sampled screenshot audits. Forced-colors behavior is browser emulation only; native high-contrast rendering, Tauri resizing, and in-game appearance remain unverified here.

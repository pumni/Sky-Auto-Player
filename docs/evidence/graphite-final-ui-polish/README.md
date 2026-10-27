# Graphite desktop UI final polish evidence

This report records visual and computed-style evidence for issue #549. The implementation source
is at `9ad0655de8fb852c5e927acd42402e374d70382d`; this report and the screenshots are committed in
a later documentation-only commit. The before capture uses
`cf97261887047c626b9222c118adab32a27e39df`.

## Capture conditions

- Host: Windows (`win32`), Chromium 156.0.8078.4, Playwright.
- Browser captures use the UI's mock playback bridge. No physical gameplay input was sent.
- Before and after browser captures were made with the same capture spec, viewport sizes, and
  built-in sample library. `before/computed-controls.json` and `after/computed-controls.json`
  contain the source revision, viewport, parent surface colors, and computed control styles.
- The after browser capture and state assertions ran against implementation SHA
  `9ad0655de8fb852c5e927acd42402e374d70382d`.
- The native Tauri caption capture was made from source SHA
  `aaebbe25b449dd894193a0774c6afd55fcd43123`, immediately before that source was committed. The
  subsequent source commit `9ad0655de8fb852c5e927acd42402e374d70382d` only darkened the
  pressed-state token; it does not affect the hovered Close styling shown. The image shows the
  native app window with Close hovered at 125% Windows display scaling (800×590 logical,
  1000×738 physical pixels). No playback was started. This verifies the native caption hover only;
  the other states below are browser/WebView test captures.

## Before and after

The idle Play control had a filled `rgb(36, 37, 38)` background before the change. At rest in the
after capture, Play is transparent (`rgba(0, 0, 0, 0)`), including while disabled. The active
Library control also changed from a filled background to a transparent surface with the Graphite
accent (`rgb(178, 165, 233)`) as its label color. Its selected state remains legible without a
permanent button fill. The after state test checks that enabled Play gains the hover token only on
hover and uses the pressed token only while pressed.

| Visual diff index                                   | Before                                                          | After                                                          |
| --------------------------------------------------- | --------------------------------------------------------------- | -------------------------------------------------------------- |
| Player idle, 800×560                                | [Screenshot](before/player-idle-800x560.png)                    | [Screenshot](after/player-idle-800x560.png)                    |
| Caption close hover crop, 46×40 (1366×768 viewport) | [Screenshot](before/titlebar-close-hover-1366x768.png)          | [Screenshot](after/titlebar-close-hover-1366x768.png)          |
| Selected track playing, 1366×768                    | [Screenshot](before/player-playing-selected-track-1366x768.png) | [Screenshot](after/player-playing-selected-track-1366x768.png) |
| Settings, 1366×768                                  | [Screenshot](before/settings-1366x768.png)                      | [Screenshot](after/settings-1366x768.png)                      |
| Inspector overlay, 640×448                          | [Screenshot](before/inspector-overlay-640x448.png)              | [Screenshot](after/inspector-overlay-640x448.png)              |
| Runtime Inspector, 1920×1080                        | [Screenshot](before/runtime-1920x1080.png)                      | [Screenshot](after/runtime-1920x1080.png)                      |

The titlebar files are 46×40-pixel control crops from the 1366×768 browser viewport. The other
matched hover details are [minimize](before/titlebar-minimize-hover-1366x768.png) / [after](after/titlebar-minimize-hover-1366x768.png),
[maximize](before/titlebar-maximize-hover-1366x768.png) / [after](after/titlebar-maximize-hover-1366x768.png), and
[inactive maximize](before/titlebar-inactive-maximize-hover-1366x768.png) / [after](after/titlebar-inactive-maximize-hover-1366x768.png).

## State coverage

The matched browser captures also include:

- Player paused and playing with a selected track.
- Keyboard focus on a player utility control.
- Minimize, maximize, inactive maximize, and close hover states.
- Docked Inspector at 1366×768 and overlay Inspector at 640×448.
- New playlist dialog, playlist action menu, and delete confirmation.
- Idle player at 800×560 and Runtime Inspector at 1920×1080.

The corresponding before and after PNGs are in the `before/` and `after/` folders. Computed
measurements for the idle player, player states, focus, Inspector layouts, Runtime, and dialogs are
in the respective `computed-controls.json` files. These files record visible control dimensions,
colors, borders, focus outline, disabled state, and surrounding surface colors.

The screenshot matrix uses DPR 1 at 640×448 (Inspector overlay), 800×560 (idle player), 1366×768
(player, titlebar, settings, dialogs, and docked Inspector), and 1920×1080 (Runtime). The separate
titlebar regression checks pointer operability, geometry, and no horizontal overflow at 640×448,
800×560, 1024×640, 1366×768, and 1920×1080, at DPR 1, 1.25, and 1.5, with Update both present and
absent.

The `graphite-final-ui-polish.spec.ts` regression enumerates visible button-like controls in the
captured states and checks transparent rest, disabled opacity of `1`, disabled Play remaining
unchanged on hover, enabled Play hover and pressed fills, transient caption and utility fills,
inactive caption hover, danger close, accent selected navigation, visible keyboard focus,
danger-action hover, and computed contrast. It also emulates forced-colors and reduced-motion
modes. Shared reduced-motion styles remain in effect; this change adds no motion. Geometry, target
sizes, compact no-overflow behavior, and the five other palette contracts remain covered by the
Graphite geometry, titlebar, and palette regressions.

## Contrast matrix

Ratios below use WCAG relative luminance on Graphite CSS token values. Playwright asserts the
accent, danger, hover, pressed, and focus pairs against actual computed styles. Text uses the 4.5:1
target; key focus and non-text indicators use 3:1.

| Role and state               | Foreground | Background             | Contrast |
| ---------------------------- | ---------- | ---------------------- | -------: |
| Primary text at rest         | `#d4d4d4`  | `#151618` content      |  12.22:1 |
| Secondary text at rest       | `#a6a6a6`  | `#191a1b` navigation   |   7.16:1 |
| Accent selected text         | `#b2a5e9`  | `#191a1b` navigation   |   7.84:1 |
| Accent on hover              | `#b2a5e9`  | `#35363a` hover        |   5.43:1 |
| Accent on pressed            | `#b2a5e9`  | `#404246` pressed      |   4.53:1 |
| Hover accent on hover        | `#c0b4f1`  | `#35363a` hover        |   6.34:1 |
| Focus ring on navigation     | `#b2a5e9`  | `#191a1b` navigation   |   7.84:1 |
| Destructive action on hover  | `#f27a80`  | `#452d32` danger hover |   4.70:1 |
| Caption close glyph on hover | `#ffffff`  | `#c42b36` danger fill  |   5.61:1 |
| Disabled label visibility*   | `#8b8c90`  | `#191a1b` navigation   |   5.19:1 |

\* Disabled controls remain fully opaque and do not respond to hover. WCAG exempts inactive
controls from the text contrast minimum; this ratio is included as visibility context.

## Evidence limits

Browser screenshots and computed styles establish the UI states exercised with the mock bridge;
they do not establish native playback or physical input behavior. The native Tauri image is limited
to the caption close hover state and its source revision is stated above. The full `cargo xtask
check all` gate passed at implementation SHA `9ad0655de8fb852c5e927acd42402e374d70382d`; the
conditional capture and state tests were also run separately at that SHA (2 passed). This task
changes desktop presentation and tests only; native playback and security behavior were not
changed.

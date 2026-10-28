# Issue 554 B1/B2 evidence — `f07a2eb`

These captures and measurements are tied to product source revision
`f07a2ebaf561918b90567e49bbc69b611618ad56` on branch
`codex/ui-unification-workbench-performance-20261027`. The existing files at the parent directory
root remain the historical `c96a9d9` captures.

## B1 — live Win32 smoke

The live target was `rust/target/debug/sky_desktop_shell.exe`, SHA-256
`1C394BCFF7E2B115CB0B4633BDFF8D2D3B017043AC5D9C4EEF57CF01F640EB2F`, launched with
`bun run tauri:dev`. The host was Windows 11 Home build 26300. `GetDpiForWindow` returned 120 DPI
(125%); the display's physical bounds were 1920×1080 and the live window client used a 40 logical px
titlebar, equal to 50 physical px at this DPI.

| Action | Native observation |
| --- | --- |
| Snap Layout hover | Hover at physical screen point `(1818, 348)` hit `SkyAutoPlayerSnap`; Windows Snap Layout appeared in [the live capture](native-125-snap-hover.png). |
| Click just below titlebar | A real Win32 `SendInput` left click at `(1413, 374)` was 51 physical px below the client origin `(922, 323)`: 50 px titlebar plus 1 px. `WindowFromPoint` returned `Chrome_RenderWidgetHostHWND` from `msedgewebview2.exe`, whose parent was `sky_desktop_shell.exe`; the focused UIA element was the `Sky Auto Player` document. |
| Maximize | Clicking the native maximize hit target changed `IsZoomed` to `true`; outer rect became `-9,-9,1929,1089`. |
| Restore | Clicking the same control changed `IsZoomed` to `false`; outer rect returned to `913,322,1913,1022`. |
| Minimize and restore | The native minimize button changed `IsIconic` to `true`. `ShowWindow(SW_RESTORE)` returned the window to its normal, visible state. |
| Drag and focus recovery | The window moved and stayed responsive; foreground returned to the app and UIA focus was `Minimize window` (`Button`). Two native input drivers produced different window-to-pointer deltas; see `dragInput` and `dragDistanceCrossCheck` in [the detailed result](native-125-acceptance.json). The drag completed, but these measurements do not establish 1:1 pointer-to-window travel. |
| Monitor change | N/A: one display (`0,0,1920,1080`). |
| Other DPI scales | Not available in this session; 100%, 150%, and 200% remain untested. |

The click and caption actions used Win32 input against the app's own window. No game was launched and
no gameplay input was sent. The JSON records the HWND/process relationship, executable hash, DPI,
physical rectangles, focus observation, and individual outcomes.

## B2 — source-matched browser captures and palette parity

The [capture manifest](browser-mock-capture-manifest.json) tags the three browser mock screenshots
and [geometry measurements](browser-mock-geometry.json) with source revision
`f07a2ebaf561918b90567e49bbc69b611618ad56`:

| Capture | Viewport | State |
| --- | ---: | --- |
| [Populated workbench](browser-mock-1366x768-populated.png) | 1366×768 CSS px | Expanded navigator, selected track, dry-run playback, docked Inspector |
| [Compact workbench](browser-mock-800x560-compact.png) | 800×560 CSS px | Collapsed navigator, selected track, dry-run playback |
| [Inspector overlay](browser-mock-640x448-overlay.png) | 640×448 CSS px | Selected track, dry-run playback, Inspector overlay |

The new [Settings modal parity test](../../../../desktop/tests/e2e/palette-appearance.spec.ts)
checks modal and child rectangles, accessible roles/names, six-palette geometry within 0.5 CSS px,
and the keyboard focus order. Its [recorded matrix](settings-modal-six-palette.json) is tagged with
the same source SHA and includes one screenshot per palette at 800×560 CSS px. All six palettes had
the same measured rectangles and focus order:

`Close settings → Playback → Appearance → Diagnostics → Updates → Advanced → About → Color palette → Close settings`

The dialog surface color changed with each palette. Closing the dialog restored focus to the
`Open settings` trigger.

Capture command, from `desktop/`:

```powershell
$env:GRAPHITE_WORKSPACE_CAPTURE='candidate'
$env:GRAPHITE_WORKSPACE_CAPTURE_VIEWS='populated,compact,overlay,modal'
$env:GRAPHITE_EVIDENCE_SOURCE_SHA='f07a2ebaf561918b90567e49bbc69b611618ad56'
bun run test:e2e -- tests/e2e/visual-evidence.spec.ts tests/e2e/palette-appearance.spec.ts --grep "captures selected Graphite workspace evidence views on request|Settings modal geometry, roles, and focus order are shared across palettes" --workers=1
```

## Validation record

All commands below ran on working `HEAD` `f07a2ebaf561918b90567e49bbc69b611618ad56`; the added
test file was `desktop/tests/e2e/palette-appearance.spec.ts`, blob
`8b6a088892d532c75ac5fb5944237cbed007f7e6`. The application executable used for B1 has the SHA-256
recorded above. The test file and evidence are committed after validation without further source
changes.

| Command | Exit | Coverage |
| --- | ---: | --- |
| Capture command above | 0 | 2 E2E tests passed; generated the three workbench captures, geometry JSON, and six modal screenshots/matrix |
| `bun run test:e2e -- tests/e2e/palette-appearance.spec.ts --grep "Settings modal geometry, roles, and focus order are shared across palettes"` | 0 | 1 modal parity E2E test passed |
| `bun run typecheck` | 0 | TypeScript |
| `bunx eslint tests/e2e/palette-appearance.spec.ts --max-warnings 0` | 0 | New E2E test |
| `bunx prettier --check tests/e2e/palette-appearance.spec.ts` | 0 | New E2E test formatting |
| `cargo xtask check all` | 0 | Static, Rust, desktop/full E2E, and native desktop checks passed |
| `cargo xtask check static` | 0 | Documentation integrity, security, and static contracts passed after the evidence README update |
| `git diff --check` | 0 | No whitespace errors |

The browser captures and accessibility/geometry checks are mock evidence; they do not replace the
live Win32 smoke above.

# Graphite Workbench — Visual Approval Gate #2

**Owner decision required before merge:** please approve the V2 Graphite result or request changes using the screenshot names and elements below. Visual Gate #1 approved the direction; it did not approve this final V2 result.

## Matched captures

The comparison uses the approved V1 source at `e91c1562e748f6c9f0be40df33d6564dc423e447` and the same Playwright evidence flow for both variants. The V1 screenshots set the existing non-persisted Graphite preview on the document. V2 uses the persistent Graphite palette as the browser mock's new-install default.

| CSS viewport | V1 Graphite preview | V2 Graphite |
| --- | --- | --- |
| 640 × 448 | [Open capture](graphite-v1/populated-640x448.png) | [Open capture](graphite-v2/populated-640x448.png) |
| 800 × 560 | [Open capture](graphite-v1/populated-800x560.png) | [Open capture](graphite-v2/populated-800x560.png) |
| 1024 × 640 | [Open capture](graphite-v1/populated-1024x640.png) | [Open capture](graphite-v2/populated-1024x640.png) |
| 1366 × 768 | [Open capture](graphite-v1/populated-1366x768.png) | [Open capture](graphite-v2/populated-1366x768.png) |
| 1920 × 1080 | [Open capture](graphite-v1/populated-1920x1080.png) | [Open capture](graphite-v2/populated-1920x1080.png) |

Each populated capture shows the Update control, focused Search, the 500-track browser, Song Details, and Candle Run playing while Blue Bird is selected for inspection. The 640 and 800 captures use the compact Inspector overlay; the wider captures use the docked Inspector. PNG dimensions were checked against each named viewport.

### State comparisons

| State | V1 Graphite preview | V2 Graphite |
| --- | --- | --- |
| Empty library | [800 × 560](graphite-v1/empty-library-800x560.png) | [800 × 560](graphite-v2/empty-library-800x560.png) |
| Empty playlist | [800 × 560](graphite-v1/empty-playlist-800x560.png) | [800 × 560](graphite-v2/empty-playlist-800x560.png) |
| Imported playlist | [1024 × 640](graphite-v1/imported-playlist-1024x640.png) | [1024 × 640](graphite-v2/imported-playlist-1024x640.png) |
| Dry-run Runtime, including “no physical input sent” | [640 × 448](graphite-v1/runtime-dry-run-640x448.png) · [1366 × 768](graphite-v1/runtime-dry-run-1366x768.png) | [640 × 448](graphite-v2/runtime-dry-run-640x448.png) · [1366 × 768](graphite-v2/runtime-dry-run-1366x768.png) |
| Collapsed Navigator | [1366 × 768](graphite-v1/navigator-collapsed-1366x768.png) | [1366 × 768](graphite-v2/navigator-collapsed-1366x768.png) |
| Keyboard-focused contextual menu | [1366 × 768](graphite-v1/context-menu-keyboard-focus-1366x768.png) | [1366 × 768](graphite-v2/context-menu-keyboard-focus-1366x768.png) |
| Keyboard-focused Appearance setting | [1366 × 768](graphite-v1/settings-appearance-keyboard-focus-1366x768.png) | [1366 × 768](graphite-v2/settings-appearance-keyboard-focus-1366x768.png) |
| Pending playback confirmation and timing warning | [1366 × 768](graphite-v1/pending-playback-confirmation-1366x768.png) | [1366 × 768](graphite-v2/pending-playback-confirmation-1366x768.png) |
| Target recovery warning | [800 × 560](graphite-v1/playback-recovery-800x560.png) | [800 × 560](graphite-v2/playback-recovery-800x560.png) |
| Uncertain native status with Retry and Stop | [1024 × 640](graphite-v1/uncertain-native-status-1024x640.png) | [1024 × 640](graphite-v2/uncertain-native-status-1024x640.png) |
| Physical-mode UI with degraded diagnostics | [1366 × 768](graphite-v1/physical-playback-degraded-diagnostics-1366x768.png) | [1366 × 768](graphite-v2/physical-playback-degraded-diagnostics-1366x768.png) |

### Close-ups

Titlebar crops are available at all five viewport widths: [V1](graphite-v1/closeup-titlebar-640x448.png) · [V2](graphite-v2/closeup-titlebar-640x448.png), [V1](graphite-v1/closeup-titlebar-800x560.png) · [V2](graphite-v2/closeup-titlebar-800x560.png), [V1](graphite-v1/closeup-titlebar-1024x640.png) · [V2](graphite-v2/closeup-titlebar-1024x640.png), [V1](graphite-v1/closeup-titlebar-1366x768.png) · [V2](graphite-v2/closeup-titlebar-1366x768.png), [V1](graphite-v1/closeup-titlebar-1920x1080.png) · [V2](graphite-v2/closeup-titlebar-1920x1080.png).

Additional crops compare the Inspector overlay ([V1](graphite-v1/closeup-inspector-overlay-640x448.png), [V2](graphite-v2/closeup-inspector-overlay-640x448.png)), docked Inspector ([V1](graphite-v1/closeup-inspector-docked-1366x768.png), [V2](graphite-v2/closeup-inspector-docked-1366x768.png)), selected row ([V1](graphite-v1/closeup-row-selected-1366x768.png), [V2](graphite-v2/closeup-row-selected-1366x768.png)), Now Playing row ([V1](graphite-v1/closeup-row-playing-1366x768.png), [V2](graphite-v2/closeup-row-playing-1366x768.png)), and Player Dock ([V1](graphite-v1/closeup-player-dock-1366x768.png), [V2](graphite-v2/closeup-player-dock-1366x768.png)).

## Verification

- `cargo xtask check all` — passed at source commit `85fd4dedb3354f98210e581c3aa608638c582e46`, including static/security, Rust, full desktop E2E, and native desktop checks.
- `bun run test:e2e:bundle` — passed at the same source commit; the production bundle booted and completed the representative browser flow.
- `bun run test:e2e visual-workbench-evidence.spec.ts --reporter=line` — passed for both V1 (`GRAPHITE_WORKBENCH_VARIANT=graphite-v1`) and V2 (`GRAPHITE_WORKBENCH_VARIANT=graphite-v2`); 27 captures per variant. V2 was refreshed from `85fd4dedb3354f98210e581c3aa608638c582e46`.
- Focused browser runs passed for titlebar geometry, palette contrast and forced colors, compact/docked Inspector behavior, and the updated populated-library and Player Dock interaction expectations.
- Focused settings tests passed for Graphite as the missing-file default, previous saved palette round-trips, and Aurora fallback for existing settings with no valid palette.

## Design decisions

- **Palette:** neutral graphite surfaces, cool high-contrast text, fine gray borders, and a restrained purple accent for active selection, focus, tabs, and primary controls. [Palette swatches](graphite-palette-swatches.svg).
- **Text hierarchy:** song titles and primary content use 13–14 px text; metadata and supporting diagnostics use 11–12 px. Section labels are quieter while status values and warnings remain easy to find.
- **Component emphasis:** the Navigator, Track Browser, and Inspector now share one workbench surface with subtle dividers. The Inspector reads as a slightly raised utility pane; the compact overlay retains a clear edge and shadow. The Player Dock keeps its fixed transport layout.
- **Selection and playback:** a selected song uses the purple selection surface and edge. Now Playing keeps a speaker marker and a separate status in the dock, so playback identity does not depend on color alone.
- **Density and balance:** 46 px song rows, the 92 px Player Dock, resizable pane geometry, and the 280 px usable Inspector minimum remain intact. Repeated playlist instructions disappear from populated libraries while create/import actions remain available.

### Graphite token swatches

| Canvas | Workbench panel | Inspector | Raised control | Selection |
| --- | --- | --- | --- | --- |
| `#121416` | `#191C20` | `#16191D` | `#25292F` | `#272632` |

| Primary text | Secondary text | Subtle border | Strong border | Accent / focus |
| --- | --- | --- | --- | --- |
| `#DCE1E8` | `#A9B2BD` | `#2C3036` | `#394049` | `#7663B8` / `#A99AF7` |

## Capture provenance and limits

- V1 appearance source: exact V1-approved baseline commit `e91c1562e748f6c9f0be40df33d6564dc423e447`.
- V2 implementation source: `85fd4dedb3354f98210e581c3aa608638c582e46`; the V2 screenshots were refreshed from this exact source commit.
- Both sets were captured by Playwright from the application's actual React workbench, with identical viewport sizes, mock catalog content, and state sequences. They are not synthetic UI mockups.
- The detached V1 capture checkout received temporary browser-mock fixture options for empty-catalog and degraded-diagnostics states. That temporary patch changed no visual components, styles, or production behavior.
- The evidence browser uses the mock bridge. “Physical playback” and degraded diagnostics show the real UI state with deterministic mock data; the capture does not send physical keyboard input. Dry-run captures explicitly show `Mode: Dry run · no physical input sent`.
- V1 Graphite was only a CSS preview. Its Settings screenshot therefore still shows Aurora selected in persisted settings; V2 shows Graphite as the saved palette. This difference is part of the palette-integration change.
- At 640 CSS px, compact spacing puts Playing below Now Playing and Dry run, keeping all three legible within the fixed 92 px Player Dock. Runtime carries the full no-physical-input explanation and status detail.
- Browser captures do not replace the separate titlebar geometry, pointer-hit, forced-colors, and responsive Playwright checks; they also do not reproduce OS window shadows or hardware DPI rasterization.

## Owner approval requested

Please explicitly approve Visual Gate #2, or request specific changes to:

1. The integrated workbench surfaces, text hierarchy, and restrained Graphite accent.
2. The difference between selected and Now Playing rows, including the docked and overlay Inspector states.
3. The Graphite Settings choice and new-install default shown in the V2 Settings capture.
4. The 640 px titlebar, overlay, Player Dock, and no-input Runtime presentation.
5. The warning, recovery, uncertain-status, and degraded-diagnostics hierarchy.

If changes are requested, reference the screenshot name and the affected element. This package does not authorize merge; Visual Gate #2 and technical acceptance are both required first.

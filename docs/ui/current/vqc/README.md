# VQC-R2 visual evidence

Captured from desktop source [`4615bbb722d9`](https://github.com/pumni/Sky-Auto-Player/commit/4615bbb722d9a133d1c0a9b3dd8c74620a208600) (desktop tree `9d789c46bc45214a97af6e3f881c6b4266a78aad`) with Playwright Chromium and the deterministic browser mock bridge. These are rendered app states, not native Tauri or gameplay evidence. The Profile captures use 800x560; the compact Settings captures use 640x448 and 640x320.

| Capture | Rendered state |
| --- | --- |
| [tooltip-open.png](tooltip-open.png) | Settings keyboard-shortcut tooltip on the docked Workbench, 1366x768 |
| [scrollbar-idle-workbench.png](scrollbar-idle-workbench.png) | Workbench with the focused scrollbar fully hidden at idle |
| [scrollbar-visible-scrolling.png](scrollbar-visible-scrolling.png) | The same Workbench during actual wheel scrolling |
| [scrollbar-visible-dragged.png](scrollbar-visible-dragged.png) | The same Workbench while the thumb remains actively dragged after scrolling settles |
| [profile-expanded-800x560.png](profile-expanded-800x560.png) | Playback Profile with Advanced timing expanded at 800x560; all fields fit without scrolling |
| [profile-select-open-800x560.png](profile-select-open-800x560.png) | Playback Profile Base Hold Select open at 800x560 (Graphite) |
| [profile-select-open-light-800x560.png](profile-select-open-light-800x560.png) | Playback Profile Base Hold Select open at 800x560 (Light) |
| [settings-categories-640x448.png](settings-categories-640x448.png) | All six vertical Settings categories visible at 640x448 |
| [settings-categories-640x320-scrolled.png](settings-categories-640x320-scrolled.png) | Settings category navigation scrolled at 640x320 |
| [settings-select-open-640x320.png](settings-select-open-640x320.png) | Settings Dark palette Select open at 640x320 |
| [inspector-details-tab.png](inspector-details-tab.png) | Inspector Details selected with the Readiness state visible |
| [inspector-runtime-performance.png](inspector-runtime-performance.png) | Runtime Performance tab in deterministic dry-run browser state |
| [inspector-runtime-timing.png](inspector-runtime-timing.png) | Runtime Timing tab with the segmented indicator moved to Timing |
| [inspector-events-tab.png](inspector-events-tab.png) | Runtime Events with full-width detail rows beneath their timestamps and names |
| [light-workbench.png](light-workbench.png) | 100% undimmed Light Workbench with selected/playing/hovered rows, Runtime indicator, and secondary action |
| [palette-comparison-contact-sheet.png](palette-comparison-contact-sheet.png) | Rendered Settings Appearance comparison for Light and all six dark palettes |
| [workbench-palette-contact-sheet.png](workbench-palette-contact-sheet.png) | Undimmed Workbench comparison across Light and all six dark palettes (selected, playing, hovered rows, Runtime indicator, Player, secondary action) |
| [transport-play-idle.png](transport-play-idle.png) | Idle transport with a selected short title and truthful Selected status |
| [transport-pause-playing.png](transport-pause-playing.png) | Filled Pause control while browser-mock dry-run playback is active |
| [transport-resume-paused.png](transport-resume-paused.png) | Filled Resume control while browser-mock dry-run playback is paused |
| [transport-long-title-compact.png](transport-long-title-compact.png) | Compact Player Dock with a long title and readable selected/prepared status |

The palette contact sheet contains the Settings Appearance view in Light and each dark palette. The separate interactive checks cover idle, wheel, keyboard, settled, active drag, release, cancellation, no-overflow unmount, reduced-motion, and forced-colors behavior.

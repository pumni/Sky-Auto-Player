# PR #238 / Issue #237 — Player UX Tauri evidence

Captured from implementation source commit `dcfd670ecdec75d2805518da9b9fe827402d7027` in a real Windows Tauri/Wry window using Microsoft Edge WebView2 `154.0.4258.12`. Playwright connected to the WebView2 local CDP endpoint; these are not browser-mock captures. `geometry.json` records the runtime, viewport, document bounds, UI state, measured row geometry, and SHA-256 for each PNG.

At 125% display scaling (`devicePixelRatio = 1.25`), the wide WebView viewport was `960×608` and the compact viewport was `640×448`. The corresponding PNG surfaces are `1200×760` and `800×560`. Each capture reports that the document matches its viewport and the workbench fits. The playing row remains 46 CSS px high in wide and compact captures; selected and playing rows retain the same height when they differ. `align: 'auto'` following is covered by the regression suite.

Player screenshots use **Test playback (no input)**. The `next-song-now-playing` capture follows explicit Next during Test playback: We Wish You A Merry Christmas is Now Playing while On My Way remains selected. The natural Auto Play handoff is covered by store and E2E tests; it is not represented as an actual-Tauri advancing-state screenshot because dry-run Test playback intentionally remains one-shot, and exercising non-dry-run playback would send gameplay input.

| Capture | State | SHA-256 |
| --- | --- | --- |
| `playing-selected-and-now-playing-1200x760.png` | All Of Me selected and playing | `D93088E5A0BC932A25F1A5F5ADB3F9DB6A46F7B4F14A4CA8E49E1DCEFD0BB3D8` |
| `playing-different-from-selected-1200x760.png` | All Of Me playing; On My Way remains selected | `B7682E90B04239BAE575D5000FFBFE421E3AF9CC43F05A6675AEB583662AF7E9` |
| `next-song-now-playing-1200x760.png` | We Wish You A Merry Christmas Now Playing after explicit Next; On My Way remains selected | `0D52F7A11D7B5333A80B1CB1B1B4136040FB55CAB70DB30EBE1B5E1A0DBE11BD` |
| `autoplay-icon-off-1200x760.png` | Right-dock ListMusic Auto Play button Off | `C122401E93E11C369A168D7ADBE8D758AAF80677C70BC91740B2CBE920681285` |
| `autoplay-icon-on-1200x760.png` | Right-dock ListMusic Auto Play button On | `4827638B59A2253625788EF79A84114B95A7E5B905C805D2FC33EED8A6E3D27C` |
| `settings-playback-behavior-1200x760.png` | Settings > Playback with Behavior and Auto Play | `B27D157AB6718CAA91D6C932BF89563895FBBB03ABCE2041B45BD45DF05C9A05` |
| `compact-library-now-playing-800x560.png` | Compact Library/Player with Now Playing row visible | `AB40398D8D6A30FC40FAF97DD38A89974639195F332C70C719C42499A0F005B0` |
| `compact-settings-playback-behavior-800x560.png` | Compact Settings > Playback with wrapped description | `263B2E2E809816FDAA6EFD323B4E9BAA5403F6AEFF0B68653D43267732DEED4E` |

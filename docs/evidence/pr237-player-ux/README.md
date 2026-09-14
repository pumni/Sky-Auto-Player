# PR #237 — Player UX Tauri evidence

Captured from implementation source SHA `60836de312382954b35937c47b88a3a874ee13f6` in a real Windows Tauri/Wry window using Microsoft Edge WebView2 `154.0.4258.12`. Playwright connected to the WebView2 local CDP endpoint; these are not browser-mock captures. `geometry.json` records the runtime, viewport, document bounds, UI state, measured row geometry, and SHA-256 for each PNG.

At 125% display scaling (`devicePixelRatio = 1.25`), the wide WebView viewport was `960×608` and the compact viewport was `640×448`. The corresponding PNG surfaces are `1200×760` and `800×560`. Each capture reports that the document matches its viewport and the workbench fits. The playing row remains 46 CSS px high in wide and compact captures; selected and playing rows retain the same height when they differ. `align: 'auto'` following is covered by the regression suite.

Player screenshots use **Test playback (no input)**. The `next-song-now-playing` capture follows explicit Next during Test playback and shows the new current song in the Library while the selected row remains independent. The natural Auto Play handoff is covered by store and E2E tests; it is not represented as an actual-Tauri advancing-state screenshot because dry-run Test playback intentionally remains one-shot, and exercising non-dry-run playback would send gameplay input.

| Capture | State | SHA-256 |
| --- | --- | --- |
| `playing-selected-and-now-playing-1200x760.png` | Same row selected and playing | `2E8E999DCA6B932D4643F04C83C5CCBF928F7C57C2D50D84A8765ACECA0AF530` |
| `playing-different-from-selected-1200x760.png` | Senorita playing; Servant of Evil remains selected | `72C2D3BE6AA27613E82E79B8C48C7FA24F260094334CCED0448C2AAF77C58D59` |
| `next-song-now-playing-1200x760.png` | Ocean Eyes Now Playing after explicit Next; selected row is unchanged | `A2AF329429B6673CC70BFD6A91F3F6DE7DAD85832A8E36B4EFEC6070D9A20B05` |
| `autoplay-icon-off-1200x760.png` | Right-dock ListMusic Auto Play button Off | `740CD67D57B8E25E3C22212D6A9641D6C311965A8BB802ADCDFACFCBC7DA4731` |
| `autoplay-icon-on-1200x760.png` | Right-dock ListMusic Auto Play button On | `AD58B6D10AC5775C4898B6A14B2FB9A1110705EE064CEE798457853F20E3E578` |
| `settings-playback-behavior-1200x760.png` | Settings > Playback with Behavior and Auto Play | `3CD03F30580E703A62CAC08146C620F23FF8ECBFC1818C129410B332E5A80D89` |
| `compact-library-now-playing-800x560.png` | Compact Library/Player with Now Playing row visible | `83B6265FEEEAC08C1C17414DEC4DF2694D68840D45F46892B6F7837043237AD` |
| `compact-settings-playback-behavior-800x560.png` | Compact Settings > Playback with wrapped description | `DBE7B42CA033CA329AE1E146991539BB6451488462FD4498315C62DDDEF5C74F` |

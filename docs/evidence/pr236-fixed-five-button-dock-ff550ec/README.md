# PR #236 — fixed transport and Auto Play dock evidence

Captured from implementation source SHA `ff550ec8f3035497a858138089263796728905a6` on the PR branch. The app was running as a real Windows Tauri/Wry window using the native bridge and WebView2 runtime `154.0.4258.12`. The captures came from that window's WebView2 surface over its local debugging endpoint; this is not a browser-shell or mock-bridge run.

Window client sizes are physical pixels at 125% Windows scaling (`devicePixelRatio = 1.25`). The 1200×760 client produced a 960×608 WebView viewport; 800×560 produced 640×448. Screenshot PNG dimensions were checked against the physical client size.

| Capture | State |
| --- | --- |
| `idle-1200x760.png` | Idle, no song selected, Shuffle Off, all five transport controls, Auto Play/Profile/Utility |
| `playing-shuffle-off-1200x760.png` | Dry-run Playing, Shuffle Off |
| `playing-shuffle-on-1200x760.png` | Dry-run Playing, Shuffle On |
| `playing-auto-play-off-1200x760.png` | Dry-run Playing, Auto Play Off |
| `playing-auto-play-on-1200x760.png` | Dry-run Playing, Auto Play On |
| `idle-800x560.png` | Idle at the compact physical client size, all five transport controls and right dock |
| `playing-800x560.png` | Dry-run Playing at the compact physical client size |
| `quick-profile-no-autoplay-1200x760.png` | Playback Profile open; contains timing controls and no Auto Play switch |
| `settings-playback-autoplay-on-1200x760.png` | Settings > Playback with Auto Play On, matching the dock |

The capture script verified all five transport buttons are visible in every captured state, their centers share a baseline, and the primary button stays centered. At 1200×760 its center was exactly x=480 in the 960px WebView viewport. At 800×560 it was x=319.988 in the 640px viewport (0.013px from center). The dock order was Auto Play x=848, Profile x=884, Utility x=920 at the wide viewport, and x=527.988, 563.988, 599.988 at the compact viewport. Each right-side tool shares a baseline with the other two. Document dimensions matched the viewport in both sizes, with no horizontal overflow.

The Playing captures use **Test playback (no input)**. They verify the real Tauri presentation and persisted settings path; they do not claim physical Sky playback, SendInput, or key-release qualification.

`geometry.json` records the WebView dimensions, button visibility/disabled state, labels, right-dock positions, and mode values for every image.

SHA-256 digests:

| File | SHA-256 |
| --- | --- |
| `idle-1200x760.png` | `F5CC361AB622540D3C0031A7032D91B8BBBF2588FC6A19D090F8016DB2E70140` |
| `playing-shuffle-off-1200x760.png` | `D3CD0AA55D8AB1C23F874A70F8AE0EB8E67D124A738B8DAA1A2F2639E17F301A` |
| `playing-shuffle-on-1200x760.png` | `846DF87B8D73ED86516A7031BD09810CB87CFB43450DF4D1B190A5827FBB8246` |
| `playing-auto-play-off-1200x760.png` | `CB48F9D79E48FC4C162E851CBF992935F316BA5B0AA530443D218FE75922E001` |
| `playing-auto-play-on-1200x760.png` | `DB63BB3E64B9AA21206BD5B403A7B09AEE94F2D8E5CAD45A9501F9FF9D823CA6` |
| `idle-800x560.png` | `9AD87CB45DFD68ED10766F17AB399A2CAD82F3F0DCF32A4C22B1C0634FE09535` |
| `playing-800x560.png` | `724A9D9D174D63FB77B02B44D1B2A212C4BF28E156613539C156DB2AE6162830` |
| `quick-profile-no-autoplay-1200x760.png` | `395C486BF33FC718528F43DA7B74A662C884E424BC8BE9BE96F23A28ADA26AD2` |
| `settings-playback-autoplay-on-1200x760.png` | `046F97A06D1A964F12D63E3B3AE528B307537F1A9FD58B2426A30334CA717158` |

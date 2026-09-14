# PR #236 — Previous boundary Tauri evidence

Captured from implementation source SHA `da972e5a5fd6ec84f76d8935f9c6ded4ff88a5a0`. The app was running as a real Windows Tauri/Wry window with WebView2 runtime `154.0.4258.12`; screenshots came from that window's WebView2 surface over its local debugging endpoint, not a browser shell.

The native Tauri client window was resized to physical `1200×760` and `800×560` pixels using Win32 `SetWindowPos`. At 125% Windows scaling (`devicePixelRatio = 1.25`), the corresponding WebView viewports were `960×608` and `640×448`. Each PNG's physical dimensions were read from its PNG header and checked against the requested client size. `geometry.json` contains the measured control states and geometry for every capture.

The regression-specific evidence shows Previous disabled at traversal position 0 with `102837` μs elapsed, then enabled after the 3-second threshold at `3053443` μs. The same disabled-at-origin state is captured at both client sizes. All five transport controls remain visible, share a baseline within 1 px, and the primary stays centered within 2 px. Auto Play, Profile, and Utility remain in order on a shared baseline with no document overflow.

All Playing captures use **Test playback (no input)**. They verify the real Tauri presentation and lifecycle UI, not physical Sky playback or SendInput qualification. Physical-input qualification remains separate.

| Capture | State | SHA-256 |
| --- | --- | --- |
| `idle-1200x760.png` | Idle, no song selected | `038EA5775BD3E034F6CA9DF2E276DD33917EFA8106E37B7C858CA5A7A967049C` |
| `quick-profile-no-autoplay-1200x760.png` | Quick Playback Profile open; Auto Play remains in the dock only | `75D7C6471B6AB46D2A2EC46C8054C1D2C11252374C7F2CA5E2973918C8A7839D` |
| `playing-traversal-origin-previous-disabled-1200x760.png` | Dry-run Playing at traversal origin (Previous disabled) | `FACBFEE950B7E7CCACD78DEDDAAACCC0DB65F960A92A9C95D00CD6B4450061FC` |
| `playing-after-three-seconds-1200x760.png` | Dry-run Playing after 3 seconds (Previous enabled for restart) | `A816C1E91E70660A860E5D873793AF0B9E7B2D95BC9D5223B4E72C2C4A931E0C` |
| `playing-shuffle-on-1200x760.png` | Dry-run Playing, Shuffle On | `A816C1E91E70660A860E5D873793AF0B9E7B2D95BC9D5223B4E72C2C4A931E0C` |
| `playing-shuffle-off-1200x760.png` | Dry-run Playing, Shuffle Off | `47A163A3850A87DE052F63D88BD1EB556F424440935158288FF9B200570E680C` |
| `playing-auto-play-off-1200x760.png` | Dry-run Playing, Auto Play Off | `BB0D186EFFFC0CB57E265D2630E9E20BFD33A823BB0CEEEF263B46B97FD72B28` |
| `playing-auto-play-on-1200x760.png` | Dry-run Playing, Auto Play On | `55FC0D604A41D90FC0ACECBC5CFDD43039D8622A38EFF6CB8EE0EE6FA60F2F4A` |
| `settings-playback-autoplay-on-1200x760.png` | Settings > Playback, Auto Play On | `2891144495DAF45A45434701771DBF4532B5ABFFFE9668DE00E264A87949CFB5` |
| `idle-800x560.png` | Idle at compact client size | `DC941727EC29181C1F741C39B9C60BC7D988A329367D117F90ECF2920D1D3168` |
| `playing-traversal-origin-previous-disabled-800x560.png` | Dry-run Playing at compact traversal origin (Previous disabled) | `B7A5714CC9C6B989FAA276929A5DD2E214257851DD597BB8DD4915118CD82135` |

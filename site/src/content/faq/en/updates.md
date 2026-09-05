---
key: 'updates'
locale: 'en'
order: 8
category: 'General'
question: 'How does Sky Auto Player update itself?'
---

The app checks the configured v4 channel and offers **Update and Restart**. The official Tauri
updater, mediated by the Rust-owned `UpdateService`, verifies the exact NSIS artifact through its
`.exe.sig` and runs the current-user installer. V4 has no bundled custom updater executable,
portable ZIP updater, or `MANIFEST.json.sig` contract. If an update cannot be staged, download
the canonical installer from the [dedicated v4 release authority](https://github.com/pumni/Sky-Auto-Player-Releases/releases).

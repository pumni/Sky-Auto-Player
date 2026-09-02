---
key: 'source'
locale: 'en'
order: 11
category: 'General'
question: 'Can I build Sky Auto Player from source?'
---

Yes. Clone the [repository](https://github.com/pumni/Sky-Auto-Player), install the pinned Rust
toolchain and Bun, then build the native Tauri workspace under `desktop/`. The repository's
canonical checks and release tooling are Rust/Bun based; no Python installation is required.

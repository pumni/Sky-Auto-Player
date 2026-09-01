---
key: 'source'
locale: 'en'
order: 11
category: 'General'
question: 'Can I build Sky Auto Player from source?'
---

Yes. Clone the [repository](https://github.com/pumni/Sky-Auto-Player), install the pinned Rust
toolchain and Bun, then build the native Tauri workspace under `desktop/`. The temporary
repository release scripts may use Python tooling during Wave 5, but the supported application
and packaged GUI do not require Python.

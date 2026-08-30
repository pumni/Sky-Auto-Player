---
key: 'source'
locale: 'en'
order: 11
category: 'General'
question: 'Can I build Sky Auto Player from source?'
---

Yes. Clone the [repository](https://github.com/pumni/Sky-Auto-Player), run `uv sync`, then use
`uv run python src/main.py` to launch the supported source Textual/CLI fallback. The packaged
GUI is built from the `desktop/` Tauri workspace; the repository documents the Python 3.14
free-threaded environment and doctor checks.

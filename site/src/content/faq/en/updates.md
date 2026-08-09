---
key: 'updates'
locale: 'en'
order: 8
category: 'General'
question: 'How does Sky Auto Player update itself?'
---

The app checks for new versions and offers **Open GitHub Releases**. Download the canonical ZIP,
`.zip.sha256`, and `MANIFEST.json` from the official [GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases),
verify the ZIP SHA256 when desired, extract it into a new folder, and copy your `config.json`,
`.env`, `songs/`, and `logs/` into the new folder. The public package does not include a native
updater and never performs automatic extraction or application-file replacement.

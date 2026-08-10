---
key: 'updates'
locale: 'en'
order: 8
category: 'General'
question: 'How does Sky Auto Player update itself?'
---

The app checks for new versions and offers **Update and Restart**. The bundled native updater
downloads the exact release, verifies the ZIP SHA256 and `MANIFEST.json`, preserves your
`config.json`, `.env`, `songs/`, and `logs/`, then performs a transactional replacement and
restart. If staging fails, choose **Open GitHub Releases** to download the canonical ZIP,
`.zip.sha256`, and `MANIFEST.json` from the official [GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases)
and install it into a new user-writable folder. Public binaries are intentionally unsigned.

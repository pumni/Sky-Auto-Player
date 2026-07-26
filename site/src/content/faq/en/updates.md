---
key: 'updates'
locale: 'en'
order: 8
category: 'General'
question: 'How does Sky Auto Player update itself?'
---

The running app only notifies you about a release. Close the app and run `updater.bat` to apply an update. The updater verifies the downloaded ZIP’s SHA256 sidecar before touching installed files, stages and validates the release, copies binaries transactionally with rollback, and preserves `config.json`, `.env`, `songs/` and `logs/`.

Only the update notification fields in `config.json` may be patched after a successful binary update.

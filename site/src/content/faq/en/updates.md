---
key: 'updates'
locale: 'en'
order: 8
category: 'General'
question: 'How does Sky Auto Player update itself?'
---

Choose **Update now** in the running app. It stops playback, copies the bundled native updater to
a per-run directory, exits, and lets the updater independently fetch the exact target tag. The
updater verifies the canonical ZIP’s SHA256 sidecar, archive, manifest, and Authenticode signatures
before applying a transactional copy with rollback. It preserves `config.json`, `.env`, `songs/`
and `logs/`.

Only the update notification fields in `config.json` may be patched after a successful binary update.

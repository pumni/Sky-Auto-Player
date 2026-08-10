---
key: 'storage'
locale: 'en'
order: 13
category: 'General'
question: "Where are Sky Auto Player\u2019s logs and config stored?"
---

`config.json` and `songs/` live alongside `Sky-Auto-Player.exe`. The native updater writes its
durable result under `%LOCALAPPDATA%\Sky-Auto-Player\update-state\last-result.json` and keeps
temporary run data under `%LOCALAPPDATA%\Sky-Auto-Player\update-runs\`.

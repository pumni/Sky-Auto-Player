---
key: 'storage'
locale: 'en'
order: 13
category: 'General'
question: "Where are Sky Auto Player’s logs and config stored?"
---

Application settings, cache, library data, and calibration state live under the app-data boundary in
`%LOCALAPPDATA%\io.github.pumni.skyautoplayer\`. Application logs are written to the `logs\` subdirectory.
They are strictly separated from the installer-owned application directory and are preserved across updates.

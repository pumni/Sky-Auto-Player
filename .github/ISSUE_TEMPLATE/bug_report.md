---
name: Bug report
about: Broken or misbehaving playback, timing, desktop UI, library, or update in Sky Auto Player
title: "[bug] "
labels: ["bug"]
assignees: ''
---

**Before you file**

- [ ] I searched existing issues for a duplicate.
- [ ] This is a functional bug, not a security or vulnerability disclosure. For security-sensitive findings, follow [`SECURITY.md`](../../SECURITY.md) and use the private disclosure channel.
- [ ] I removed sensitive data from any pasted logs (paths and song names are fine; credentials, tokens, and personal info are not).

**Environment**

- Install method: v4 NSIS installer / dev build (`cargo tauri dev`)
- Version (from in-app Settings / About):
- Windows version + build, display scaling %, game display mode / refresh rate:
- Keyboard layout & input mode (Key Code / Scan Code) if changed from default:

**Behaviour**

What happened, and what you expected instead.

**Steps to reproduce**

1. Song format and name (built-in song, or imported `.json` / `.skysheet` / `.txt`):
2. Settings configured (Game FPS, Hold Frame Model, Timing Margin, etc.):
3. Playback action taken (keyboard hotkey, Play button in app, etc.):
4. What happened during/after playback:

**Reproduces with defaults?**

Does the issue persist when resetting playback settings (Game FPS, Hold Frame Model, Timing Margin) to defaults?

**Diagnostics & Logs**

- [ ] In-app **Diagnostics** panel summary or screenshot
- [ ] Application logs from `%LOCALAPPDATA%\io.github.pumni.skyautoplayer\logs\` (if applicable)
- [ ] Crash details / Windows Event Viewer log (if the application terminated unexpectedly)
- [ ] Screenshot or video demonstrating the behavior

**Category** (optional)

- [ ] Timing or dispatch (late/missed notes, hold frame model, chords)
- [ ] Window focus, foreground admission, or hotkeys (pause, resume, panic key)
- [ ] Desktop UI, Library, or Settings
- [ ] Sheet import / parser (JSON, SkySheet, TXT)
- [ ] Updater (channel selection, update check, installer launch)
- [ ] Startup, installer, or packaging

**Additional context**

Any system details that might be relevant (power plan, background load, third-party overlays, etc.).

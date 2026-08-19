---
name: Bug report
about: Broken or misbehaving playback/timing, TUI, update path, or CLI in Sky Auto Player
title: "[bug] "
labels: ["bug"]
assignees: ''
---

**Before you file**

- [ ] I searched existing issues for a duplicate.
- [ ] This is a functional bug, not a P0/security finding (use the *AGENTS.md P0 report* template) and not a feature request.
- [ ] I removed private data from any pasted logs (paths and song names are fine; credentials, tokens, and personal info are not).

**Environment**

- Install method: packaged `Sky-Auto-Player-v<ver>.zip` executable / `uv run python src/main.py`
- Version (from `Sky-Auto-Player.exe --version`, or `uv run python src/main.py --version`):
- Windows version + build, display scaling %, game refresh rate:
- Keyboard layout / input scan-code mode (`--scan-code-mode`) if changed from default:
- Terminal: Windows Terminal / conhost / other, if the bug is TUI rendering:

**Behaviour**

What happened, and what you expected instead.

**Steps to reproduce**

1. Song — name or minimised `.json` placed under `songs/`:
2. Launch command / TUI flow, including relevant flags (`--fps`, `--hold-frames`, `--countdown`, `--repeat`, hotkeys…):
3. What the HUD showed during/after the failure:

**Reproduces with defaults?**

Does it still happen with `--hold-frames` and `--fps` at defaults, and with the game window focused immediately after pressing the refocus key? State how you tested.

**Diagnostics** (attach, don't paste in full)

- [ ] `--doctor` / `--doctor-input` / `--doctor-calibrate` output if it throws or reports degraded
- [ ] `--debug-csv` output: the matched `.csv` and `.summary.json` from `logs/`
- [ ] `--debug-playback` log from `logs/`
- [ ] Crash log `logs/crash_*.log` if the app terminated
- [ ] Screenshot / plain-text render if the TUI looks wrong

**Category** (optional)

- [ ] Timing or dispatch (late/missed/cut notes, hold-frame, chord)
- [ ] Focus or hotkey (refocus, pause, panic, key steal)
- [ ] HUD / TUI / theme rendering
- [ ] Song decode / calibration / estimator
- [ ] Update path (notify, **Update now**, version compare)
- [ ] CLI / config / packaging

**Additional context**

Any one-off environment detail that might be relevant — noise, power plan, background load, other windows with hooks, etc.

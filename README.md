<div align="center">

# 🎵 Sky Auto Player

*Auto-plays Sky music sheets on Windows — notes, chords and holds land on the beat, not just on keypress.*

<a href="https://github.com/pumni/Sky-Auto-Player/releases/latest"><img src="https://img.shields.io/github/v/release/pumni/Sky-Auto-Player?style=for-the-badge&label=version&color=blue&logo=python" alt="Latest version"></a>
<a href="https://github.com/pumni/Sky-Auto-Player/releases"><img src="https://img.shields.io/github/downloads/pumni/Sky-Auto-Player/total?style=for-the-badge&label=downloads&logo=github&color=success" alt="Downloads"></a>
<a href="https://github.com/pumni/Sky-Auto-Player/blob/main/LICENSE"><img src="https://img.shields.io/github/license/pumni/Sky-Auto-Player?style=for-the-badge&color=orange" alt="License"></a>
<a href="https://github.com/pumni/Sky-Auto-Player/stargazers"><img src="https://img.shields.io/github/stars/pumni/Sky-Auto-Player?style=for-the-badge&label=stars&color=gold" alt="Stars"></a>

**[🌐 Landing page](https://pumni.github.io/Sky-Auto-Player/)** · **[FAQ](https://pumni.github.io/Sky-Auto-Player/faq/)** · **[Download](https://github.com/pumni/Sky-Auto-Player/releases/latest)**

</div>

<div align="center">
  <a href="site/public/assets/images/picker.webp" target="_blank">
    <img src="site/public/assets/images/picker.webp" alt="Sky Auto Player TUI picker" width="640" style="border-radius: 8px; max-width: 100%;">
  </a>
</div>

---

Sky Auto Player turns song sheets from the [specy/skyMusic](https://specy.github.io/skyMusic/)
editor into clean chords, fast arpeggios, and long holds played in-game, automatically. It
sends standard keystrokes through the public Windows `SendInput` API — the same channel any
keyboard macro uses — and never reads game memory, injects code, hooks the process, attaches a
debugger, or touches game files.

**Current source release:** `v3.0.0`.

## Why it sounds right

Sky Auto Player doesn't replay a fixed macro. It schedules every note like a small performance:

- **Contiguous chord batches** — chord notes are submitted in one `SendInput` batch to reduce sender-side skew; when the game observes them still depends on Windows and the game's sampling loop.
- **Learns your machine's latency** — adaptive lead reduces sender-side completion error from measurements on the current machine; telemetry reports the measured result instead of promising a universal absolute threshold.
- **Holds keep their full duration** — long notes don't get clipped short, even when the next note is close behind.
- **Per-song timing profiles** — fast arpeggios and slow ballads get different timing, not one global setting.
- **A dedicated Rust timing worker** — the player and the on-screen display stay on the Python UI/orchestration boundary while the real-time sender runs natively.

This is what "in time" actually means here.

> [!WARNING]
> Automated music playback may violate Thatgamecompany's Terms of Service. Use this tool
> responsibly and at your own risk.

---

## Quick start

**Requirements:** Windows 10 or 11 (64-bit). The packaged build ships its own Python — no
system Python, no installer, no admin rights, no registry entries.

1. Download `Sky-Auto-Player-v<latest>.zip` from the
   [latest release](https://github.com/pumni/Sky-Auto-Player/releases/latest).
2. Extract it anywhere — e.g. `C:\Sky-Auto-Player\`.
3. Run `Sky-Auto-Player.exe`.

### Add a song

1. Open the [Sky Music Nightly editor](https://specy.github.io/skyMusic/).
2. Export a song as **JSON**, **skysheet**, or JSON-compatible **txt**.
3. Drop the file into the `songs/` folder next to `Sky-Auto-Player.exe`.
4. Press `Ctrl+R` in the picker to reload.

---

## Features

- **Timing-first playback** — chords are submitted contiguously, holds held in full, and sender-side latency is learned and compensated; game observation still depends on Windows and the game's sampling loop
- **Textual TUI picker** — fuzzy search by song name, fully keyboard-driven
- **Per-song profiles** — timing, tempo, FPS, theme
- **Dry-run mode** — preview without sending input
- **Live HUD** — timing jitter and dispatch health at a glance
- **Tuning presets** — for weak PCs, the free-threaded `python3.14t` interpreter, and more
- **Hotkeys** — `/` command palette · `F8` pause · `F9` skip · `F10` stop · `q` / `Esc` quit

---

## Updating

Sky Auto Player checks GitHub for new releases and shows a banner when one is available. **It never
self-updates while running** — applying an update is one explicit step:

1. Close Sky Auto Player.
2. Run `updater.bat` in the install folder.
3. Reopen `Sky-Auto-Player.exe`.

> [!NOTE]
> **Pre-2.4.2 Migration (historical):** The legacy bridge update (`Sky-Player-v<ver>.zip`) was available from v2.4.2 through v2.4.4 so pre-rename `Sky-Player` installs could run their existing `updater.bat` once to migrate. The bridge was **retired in v2.4.5**. If you are still on v2.4.1 or earlier and never ran the bridge, download the canonical `Sky-Auto-Player-v<ver>.zip` manually, extract it next to the old install, and copy across your `config.json`, `.env`, `songs/`, and `logs/`.

The updater verifies SHA256 before touching any file, rolls back failed copies transactionally,
and never replaces your `config.json` or `songs/` folder. Pre-release builds:
`updater.bat -Channel beta`.

---

## FAQ

<details>
<summary><b>Will this get me banned?</b></summary>

It sends only standard keyboard input and never touches the game — no memory reads, no hooks,
no code injection, no file modification. That is the same channel any keyboard macro uses.
Automated playback may still conflict with Sky's Terms of Service, however, so use it
responsibly and at your own risk.
</details>

<details>
<summary><b>Does it run on macOS or Linux?</b></summary>

No. Sky Auto Player depends on Windows-specific APIs — `SendInput` for input simulation and MMCSS for
real-time thread scheduling. macOS and Linux are not on the roadmap.
</details>

<details>
<summary><b>Can I build it from source?</b></summary>

Yes. Clone the repo, run `uv sync` to set up the Python 3.14 free-threaded environment, then
launch with `uv run python src/main.py`. Run `--doctor` to verify your GIL state, MMCSS
availability, and key mapping, and see [docs/tuning-presets.md](docs/tuning-presets.md) for
non-standard environment presets.
</details>

The full FAQ — 14 questions covering file formats, troubleshooting, the update mechanism, and
the security model — lives at **<https://pumni.github.io/Sky-Auto-Player/faq/>**.

---

## Support

If Sky Auto Player has saved your wrist, leave a star — it helps other players find the tool. Bug
reports and ideas go to [GitHub Issues](https://github.com/pumni/Sky-Auto-Player/issues). To support
development directly:

<a href="https://ko-fi.com/pumni">
  <img src="https://ko-fi.com/img/githubbutton_sm.svg" alt="Donate on Ko-fi" width="220">
</a>

---

## License

[GNU General Public License v3.0](LICENSE) · © pumni

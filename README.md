<div align="center">

# 🎵 Sky Auto Player

*Auto-plays Sky music sheets on Windows — notes, chords, and holds land on the beat with sub-millisecond native precision.*

[![Latest version](https://img.shields.io/github/v/release/pumni/Sky-Auto-Player?style=for-the-badge&label=version&color=blue&logo=rust&logoColor=white)](https://github.com/pumni/Sky-Auto-Player/releases/latest)
[![Python 3.14t](https://img.shields.io/badge/python-3.14t%20(no--gil)-blue?style=for-the-badge&logo=python&logoColor=white)](https://docs.python.org/3.14/whatsnew/3.14.html)
[![Downloads](https://img.shields.io/github/downloads/pumni/Sky-Auto-Player/total?style=for-the-badge&label=downloads&logo=github&color=success)](https://github.com/pumni/Sky-Auto-Player/releases)
[![License](https://img.shields.io/github/license/pumni/Sky-Auto-Player?style=for-the-badge&color=orange)](https://github.com/pumni/Sky-Auto-Player/blob/main/LICENSE)
[![Stars](https://img.shields.io/github/stars/pumni/Sky-Auto-Player?style=for-the-badge&label=stars&color=gold)](https://github.com/pumni/Sky-Auto-Player/stargazers)

**[🌐 Landing Page](https://pumni.github.io/Sky-Auto-Player/)** · **[FAQ](https://pumni.github.io/Sky-Auto-Player/faq/)** · **[Download Latest](https://github.com/pumni/Sky-Auto-Player/releases/latest)**

</div>

<div align="center">
  <a href="site/public/assets/images/picker.webp" target="_blank">
    <img src="site/public/assets/images/picker.webp" alt="Sky Auto Player TUI picker" width="640" style="border-radius: 8px; max-width: 100%;">
  </a>
</div>

---

**Sky Auto Player** transforms song sheets from the [specy/skyMusic](https://specy.github.io/skyMusic/) editor into clean chords, rapid arpeggios, and expressive holds played in-game with microsecond-level timing accuracy.

The application uses a **high-performance hybrid architecture**:
- 🦀 **Native Rust Real-Time Core (`sky_player_rs`)** — Dedicated RT worker handling timeline compilation, absolute QPC scheduling, MMCSS thread priority, sub-millisecond spin-wait, focus gating, and safe input dispatch.
- 🐍 **Free-Threaded Python 3.14 (`no-GIL`)** — Responsive Textual TUI interface, song parsing, command palette, and configuration management without UI/dispatch thread contention.

Keystrokes are submitted solely through the public Windows `SendInput` API — the exact same channel used by standard keyboard macros. Sky Auto Player **never** reads game memory, injects DLLs, hooks processes, attaches debuggers, or tampers with game files.

## Why it sounds right

Sky Auto Player doesn't replay a coarse macro timer. It schedules every note like a precision musical performance:

- **Contiguous chord batches** — Chord notes are submitted in a single `SendInput` batch to minimize sender-side skew.
- **Sub-millisecond native timing** — Unshifted absolute QPC target scheduling with precision spin-wait and Windows MMCSS (`Games/Pro Audio`) registration.
- **Holds keep their full duration** — Long notes are never clipped short, even when subsequent notes follow in rapid succession.
- **Explicit hold-frame timing** — Select exact game frame holds (`1.0`, `1.25`, or `1.5` frames) tailored to the in-game FPS configuration.
- **Calibrated transport margins** — Sender-side hold shrink is calibrated and proven during admission to safeguard physical hold floors.
- **Zero GIL contention** — Python 3.14 free-threaded runtime cleanly decouples the live TUI dashboard from the native dispatch worker.

> [!WARNING]
> Automated music playback may violate Thatgamecompany's Terms of Service. Use this tool responsibly and at your own risk.

---

## Quick Start

**Requirements:** Windows 10 or 11 (64-bit). The packaged build is portable and standalone — no system Python, Rust toolchain, installer, admin rights, or registry modifications required.

1. Download `Sky-Auto-Player-v<latest>.zip` from the [latest release](https://github.com/pumni/Sky-Auto-Player/releases/latest).
2. Extract it anywhere (e.g. `C:\Sky-Auto-Player\`).
3. Run `Sky-Auto-Player.exe`.

### Adding Songs

1. Open the [Sky Music Nightly editor](https://specy.github.io/skyMusic/).
2. Export a song as **JSON**, **skysheet**, or JSON-compatible **txt**.
3. Drop the file into the `songs/` folder next to `Sky-Auto-Player.exe`.
4. Press `Ctrl+R` in the picker to reload.

---

## Features

- ⚡ **Native Rust Dispatch Core** — Absolute QPC scheduling with MMCSS audio priority and zero GC/GIL pauses
- 🎹 **Timing-first playback** — Contiguous chord batching, full hold preservation, and verified release gaps
- 🖥️ **Modern Textual TUI** — Fuzzy song search, fully keyboard-driven navigation, and live HUD
- 🎛️ **Per-song configuration** — Customizable hold profile, tempo multiplier, target FPS, and visual themes
- 🛡️ **Fail-safe controls** — Real-time focus loss detection, auto-pause, and immediate all-up key release
- 🔍 **Dry-run mode** — Preview playback rhythm in the HUD without sending keyboard input
- ⌨️ **Global Hotkeys** — `/` Command palette · `F8` Pause/Resume · `F9` Skip · `F10` Stop · `q` / `Esc` Quit

---

## Updating

Sky Auto Player automatically checks GitHub for new releases and displays a notification banner when an update is available.

Selecting **Update and Restart** launches the bundled native Rust updater (`Sky-Auto-Player-Updater.exe`), which downloads the release, cryptographically verifies the SHA256 sidecar and `MANIFEST.json`, and performs a transactional atomic replacement with automatic rollback on error. **Open GitHub Releases** remains available as a manual fallback. Preserved user data includes `config.json`, `.env`, `songs/`, and `logs/`.

> [!WARNING]
> Windows binaries in this release are intentionally unsigned and portable (no Authenticode publisher requirement). Windows SmartScreen may display an unrecognized-app warning. Download releases only from the official [GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases) and verify the published SHA256 checksums if desired.

---

## FAQ

<details>
<summary><b>Will this get me banned?</b></summary>

It sends standard keyboard input only and never touches the game process — no memory reading, no hooking, no DLL injection, and no file modification. This is identical to the mechanism used by physical programmable keyboards and macro utilities. However, automated playback may still conflict with Sky's Terms of Service, so use it responsibly and at your own risk.
</details>

<details>
<summary><b>Does it run on macOS or Linux?</b></summary>

No. Sky Auto Player depends on Windows-specific system APIs — `SendInput` for input dispatch and MMCSS / QPC for real-time thread scheduling. macOS and Linux are not supported.
</details>

<details>
<summary><b>Can I build it from source?</b></summary>

Yes. Prerequisites: Windows 10/11, [uv](https://docs.astral.sh/uv/), and the Rust toolchain (pinned in `rust/rust-toolchain.toml`).

```powershell
# 1. Clone repository
git clone https://github.com/pumni/Sky-Auto-Player.git
cd Sky-Auto-Player

# 2. Set up Python 3.14 free-threaded virtual environment
uv sync

# 3. Build and install the native Rust dispatch extension wheel
uv run python scripts/build_rust_wheel.py

# 4. Launch the application
uv run python src/main.py
```

Run `--doctor` (`uv run python src/main.py --doctor`) to verify your GIL state, native extension ABI, MMCSS availability, and key mappings.
</details>

The complete FAQ — covering supported file formats, troubleshooting, timing architecture, and the security model — is available at **<https://pumni.github.io/Sky-Auto-Player/faq/>**.

---

## Support

If Sky Auto Player has helped your musical journey, consider giving the project a star ⭐ on GitHub! For bug reports, feature suggestions, or questions, please open an issue on [GitHub Issues](https://github.com/pumni/Sky-Auto-Player/issues).

<div align="center">
  <a href="https://ko-fi.com/pumni">
    <img src="https://ko-fi.com/img/githubbutton_sm.svg" alt="Donate on Ko-fi" width="220">
  </a>
</div>

---

## License

[GNU General Public License v3.0](LICENSE) · © [pumni](https://github.com/pumni)

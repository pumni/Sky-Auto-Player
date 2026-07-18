<div align="center">

<h1>🎵 Sky Player</h1>

<p><em>An automatic music player for <b>Sky: Children of the Light</b> that actually hits the tempo you set.</em></p>

<p>
  <a href="https://github.com/pumni/Sky-Player/releases"><img src="https://img.shields.io/github/downloads/pumni/Sky-Player/total?style=for-the-badge&label=downloads&logo=github&color=success" alt="Downloads"></a>
  <a href="https://github.com/pumni/Sky-Player/releases/latest"><img src="https://img.shields.io/github/v/release/pumni/Sky-Player?style=for-the-badge&label=version&color=blue&logo=python" alt="Latest Version"></a>
  <a href="https://github.com/pumni/Sky-Player/blob/main/LICENSE"><img src="https://img.shields.io/github/license/pumni/Sky-Player?style=for-the-badge&color=orange" alt="License"></a>
  <a href="https://github.com/pumni/Sky-Player/stargazers"><img src="https://img.shields.io/github/stars/pumni/Sky-Player?style=for-the-badge&label=stars&color=gold" alt="Stars"></a>
</p>

<p>
  <a href="#-features">Features</a> ·
  <a href="#-quick-start">Quick Start</a> ·
  <a href="#-updating">Updating</a> ·
  <a href="#-faq">FAQ</a> ·
  <a href="#-license">License</a>
</p>

<a href="https://ko-fi.com/pumni">
  <img src="https://ko-fi.com/img/githubbutton_sm.svg" alt="Donate on Ko-fi" width="300">
</a>

</div>

---

Sky Player reads **JSON**, **skysheet**, or JSON-compatible **txt** song files from
[specy/skyMusic](https://specy.github.io/skyMusic/) and simulates keyboard input in real time so
you can play music sheets in Sky hands-free. It runs as a keyboard-driven
[Textual](https://textual.textualize.io/) TUI, uses about 100 MB of RAM, and is tuned to keep
timing accurate on stock CPython 3.14 on Windows.

> [!WARNING]
> Automated music playback and simulated keystrokes may violate Thatgamecompany's Terms of
> Service. Use this tool responsibly and at your own risk.

> [!TIP]
> **Friends hearing dropped notes or stuttering playback?**
> It's usually packet loss on the way to the Sky servers — the keypresses leave Sky Player on
> time, but the server never sees some of them. Install the **Cloudflare WARP client**
> (1.1.1.1) and switch to **"Traffic and DNS (UDP)"** mode. WARP's WireGuard tunnel routes game
> traffic over Cloudflare's backbone, which drops far fewer packets than most home ISPs — so the
> notes you send actually arrive.

---

## ✨ Features

- **Auto-play** — reads JSON, skysheet, or JSON-compatible txt song files
- **Real-time keypress simulation** via Windows `SendInput` only — no game tampering, memory
  reads, or injection
- **Textual TUI picker** — fuzzy search by song name, fully keyboard-driven
- **Per-song profiles** — timing, tempo, FPS, and theme controls
- **Dry-run mode** — preview a song without sending any input
- **Telemetry & HUD** — inspect timing jitter and dispatch health live
- **Tuning presets** — for weak machines, the free-threaded `python3.14t` interpreter, and more
- **Hotkeys** — `Ctrl+R` reload · `/` command palette · `q` / `Esc` quit

---

## 🚀 Quick Start

1. Download `Sky-Player-v<latest>.zip` from the
   [latest release](https://github.com/pumni/Sky-Player/releases/latest).
2. Extract it anywhere (e.g. `C:\Sky-Player\`).
3. Run `Sky-Player.exe`.

Sky Player is fully portable — it keeps everything (your profile, songs, and config) inside that
one folder, and writes no registry entries. Move or delete the folder to move or uninstall the app.

### Adding songs

1. Open [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Download a song as **JSON**, **skysheet**, or JSON-compatible **txt**.
3. Save the file into the `songs/` folder next to `Sky-Player.exe`.
4. Press `Ctrl+R` in the picker to reload the list.

---

## 🔄 Updating

Sky Player checks GitHub for new releases in the background and shows a banner when one is
available. **It never self-updates while running** — applying an update is a deliberate,
one-double-click step:

1. Close Sky Player.
2. Run `updater.bat` in the install folder.
3. Reopen `Sky-Player.exe`.

**What the updater guarantees**

- Verifies the SHA256 of the downloaded zip against its sidecar **before** touching any file.
- Checks write permission, then stages the download in `%TEMP%` and copies binaries
  transactionally — a failed copy rolls back to the previous state.
- **Never** replaces or touches your `config.json` or `songs/` folder. Your theme, timing
  profiles, and song library stay exactly as they were.
- The only config keys it may patch are `update.last_check_ts` and `update.last_notified_version`.
- Writes one line per run to `%LOCALAPPDATA%\Sky-Player\updater.log`.

**Beta channel** — run `updater.bat -Channel beta`, or set `update.channel` in `config.json` /
Update Settings, to receive pre-release builds.

> [!NOTE]
> If Windows SmartScreen warns on the first run of a new build, that is expected until code
> signing lands (tracked separately).

---

## ❓ FAQ

<details>
<summary><b>How do I update Sky Player?</b></summary>

Close the app, run `updater.bat` in the install folder, follow the prompt, then reopen
`Sky-Player.exe`. If the updater reports the app is still running, close it and re-run — or pass
`updater.bat -ForceClose` only if you accept force-stopping the process.
</details>

<details>
<summary><b>Does Sky Player self-update while running?</b></summary>

No, by design. The running app only notifies you that a new version exists; it never downloads
or replaces its own files. Running `updater.bat` is the single, explicit step that applies an
update.
</details>

<details>
<summary><b>Will updating wipe my config or songs?</b></summary>

No. The updater never replaces or touches `config.json` or `songs/`. It only patches
`update.last_check_ts` and `update.last_notified_version` in your existing config.
</details>

<details>
<summary><b>Can I move my Sky Player folder?</b></summary>

Yes. The whole folder is portable and the build writes no registry entries. Move it anywhere.
</details>

<details>
<summary><b>Where is the updater log?</b></summary>

`%LOCALAPPDATA%\Sky-Player\updater.log`. It is append-only, does not rotate, and logs a UTC
timestamp plus a short status per run — no personal information.
</details>

<details>
<summary><b>Does this work on macOS or Linux?</b></summary>

No. Sky Player targets Windows 11 and uses the Windows `SendInput` backend. Other platforms are
not supported.
</details>

<details>
<summary><b>Why does it need an ANSI terminal?</b></summary>

The picker is a Textual TUI. Use Windows Terminal, the VS Code integrated terminal, or any other
ANSI-compatible terminal. The legacy `cmd.exe` console will not render it correctly.
</details>

<details>
<summary><b>Can I tune it for a weak machine?</b></summary>

Yes. Run `--doctor` to check your GIL state, MMCSS availability, and key mapping, then pick a
preset from [docs/tuning-presets.md](docs/tuning-presets.md).
</details>

<details>
<summary><b>Is this against the TOS?</b></summary>

Automated music playback may violate Thatgamecompany's Terms of Service — use it at your
discretion. The tool performs no game memory access, injection, or anti-cheat bypass; it only
sends standard keyboard input through Windows `SendInput`.
</details>

<details>
<summary><b>Can I build from source?</b></summary>

Yes — clone the repo and run `uv sync`. See [docs/tuning-presets.md](docs/tuning-presets.md) for
non-standard environment presets.
</details>

---

## 📄 License

Sky Player is licensed under the [GNU General Public License v3.0](LICENSE).

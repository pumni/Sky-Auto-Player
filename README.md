<div align="center">
  <p align="center">
    <a href="https://github.com/pumni/Sky-Player/releases"><img src="https://img.shields.io/github/downloads/pumni/Sky-Player/total?style=for-the-badge&label=downloads&logo=github&color=success" alt="Downloads"></a>
    <a href="https://github.com/pumni/Sky-Player/releases/latest"><img src="https://img.shields.io/github/v/release/pumni/Sky-Player?style=for-the-badge&label=version&color=blue&logo=python" alt="Latest Version"></a>
    <a href="https://github.com/pumni/Sky-Player/blob/main/LICENSE"><img src="https://img.shields.io/github/license/pumni/Sky-Player?style=for-the-badge&color=orange" alt="License"></a>
    <a href="https://github.com/pumni/Sky-Player/stargazers"><img src="https://img.shields.io/github/stars/pumni/Sky-Player?style=for-the-badge&label=stars&color=gold" alt="Stars"></a>
  </p>

  # Sky Player

  <p align="center"><em>An automatic music player for <b>Sky: Children of the Light</b> that actually hits the tempo you set.</em></p>

  <a href="https://ko-fi.com/pumni">
    <img src="https://ko-fi.com/img/githubbutton_sm.svg" alt="Donate on Ko-fi" width="350">
  </a>

  ---

  <a href="#tip">Tip</a> ·
  <a href="#features">Features</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="#faq">FAQ</a> ·
  <a href="#license">License</a>

</div>

---

> [!TIP]
> **Friends hearing dropped notes or stuttering playback?**
> It's usually packet loss on the way to the Sky servers — the keypresses leave Sky Player on time, but the server never sees some of them. Try installing the **Cloudflare WARP client** (1.1.1.1) and switching to **"Traffic and DNS (UDP)"** mode. WARP's WireGuard tunnel routes game traffic over Cloudflare's backbone, which drops far fewer packets than most home ISPs — so the notes you send actually arrive.

---

Sky Player reads JSON, skysheet, or JSON-compatible txt song files downloaded from specy/skyMusic and simulates keyboard keypresses in real-time so you can play music sheets in Sky hands-free. It uses a Textual TUI interface, requires around 100mb of RAM, and is tuned to keep timing accurate on stock CPython 3.14 on Windows.

> [!WARNING]
> Automatically playing music sheets or using simulated keystrokes might violate Thatgamecompany's Terms of Service. Use this tool responsibly and at your own risk.

---

## Features

- **Auto-play** — reads JSON, skysheet, or JSON-compatible txt song files
- **Real-time keypress simulation** via Windows `SendInput` only (no game tampering)
- **Textual TUI picker** — fuzzy search by song name, keyboard-driven navigation
- **Per-song profiles** — timing, tempo, FPS, and theme controls
- **Dry-run mode** — preview songs without sending input
- **Telemetry & HUD** — inspect timing jitter and dispatch health
- **Tuning presets** — for weak machines, free-threaded `python3.14t`, and more
- **Hotkeys** — `Ctrl+R` reload, `/` command palette, `q`/`Esc` quit

## Quick Start

### Portable install (recommended)

1. Download `Sky-Player-v<latest>.zip` from the [latest release](https://github.com/pumni/Sky-Player/releases/latest).
2. Extract the zip anywhere (e.g. `C:\Sky-Player\`).
3. Double-click `Sky-Player.exe`. Sky Player keeps all its files in that folder — your
   profile, your songs, and your config stay together.

> Optional Start Menu shortcut + `.skysheet` file association may ship in a later minor
> (see `docs/2026-07-18_distribution-mpv-pattern-plan.md` Phase 4). Until then, Sky Player
> is fully portable with no installer.

### Updates

- Sky Player checks GitHub for new releases in the background and shows a banner when an
  update is available. **It does NOT self-update.**
- To update: close Sky Player, run `updater.bat` in the install folder, then reopen
  `Sky-Player-v<latest>`.
- The updater verifies the SHA256 of the downloaded zip against a sidecar **before** touching
  any install files. It checks write permission inside the folder, stages in TEMP, and copies
  binaries transactionally while **completely preserving your `config.json` and `songs/` folder**.
- It does not modify or copy anything inside your `songs/` folder, ensuring your personal song collection is never touched.
- It may update only two fields in `config.json`: `update.last_check_ts` (Unix seconds) and
  `update.last_notified_version`.
- It writes a single line to `%LOCALAPPDATA%\Sky-Player\updater.log` per run.
- Users on the `beta` channel can run `updater.bat -Channel beta`. Channel selection is also
  read from `config.json` (`update.channel`) and from Update Settings in the app.
- If Windows SmartScreen warns on first run of a new build, that is expected until code
  signing lands (separate track; not part of 2.4.0).

### Adding More Songs

1. Go to [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Download a song in **JSON**, **skysheet**, or JSON-compatible **txt** format.
3. Save the file inside the `songs/` directory.
4. Press `Ctrl+R` in the picker to reload the song list.

---

## FAQ

Q: How do I update Sky Player?
A: Close Sky Player, double-click `updater.bat` in the install folder, follow the prompt,
   then reopen `Sky-Player.exe`. If the updater says the app is still running, close it and
   re-run (or use `updater.bat -ForceClose` only if you accept force-stopping the process).

Q: Does Sky Player self-update?
A: No, by design. Like mpv, Sky Player notifies you when a new version is available, but
   does NOT download or install the new version while running. Run `updater.bat` to apply
   the update — it is one double-click away.

Q: Can I move my Sky Player folder?
A: Yes. The whole folder is portable. No registry entries are written by the portable build.

Q: Will updating wipe my config or songs?
A: No. The updater never replaces or touches `config.json` or `songs/`. It only patches
   `update.last_check_ts` and `update.last_notified_version` inside your existing config.
   Your theme, timing profiles, and song library stay completely untouched.

Q: Where can I find the updater log?
A: `%LOCALAPPDATA%\Sky-Player\updater.log`. It is append-only and does not rotate. Each line
   has a UTC timestamp and a short status; no personal information is logged.

<details>
<summary><b>Does this work on macOS / Linux?</b></summary>

No. Sky Player targets Windows 11 and uses the Windows `SendInput` backend. Other platforms are not supported.
</details>

<details>
<summary><b>Why does it require an ANSI terminal?</b></summary>

The song picker is a Textual TUI app. Use Windows Terminal, the VS Code integrated terminal, or any other ANSI-compatible terminal. The legacy `cmd.exe` console will not render correctly.
</details>

<details>
<summary><b>Can I tune it for a weak machine?</b></summary>

Yes. Run `--doctor` to check your GIL state, MMCSS availability, and key mapping, then pick a preset from [docs/tuning-presets.md](docs/tuning-presets.md).
</details>

<details>
<summary><b>Is this against TOS?</b></summary>

Automated music playback in Sky may violate Thatgamecompany's Terms of Service. Use it at your discretion. The tool performs no game memory access, injection, or anti-cheat bypass — it only sends standard keyboard input through Windows `SendInput`.
</details>

<details>
<summary><b>Can I build from source?</b></summary>

Yes — clone the repo and `uv sync`. See [docs/tuning-presets.md](docs/tuning-presets.md) for non-standard environment presets.
</details>

---

## License

Licensed under the [MIT License](LICENSE).

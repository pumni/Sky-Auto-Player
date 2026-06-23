# Sky Player

An automatic music player designed for **Sky: Children of the Light** on PC. It reads JSON, skysheet, or JSON-compatible txt song files downloaded from specy/skyMusic and simulates keyboard keypresses in real-time.

[Website](https://pumni.github.io/Sky-Player/) ·
[GitHub Repository](https://github.com/pumni/Sky-Player) ·
[Download Latest Release](https://github.com/pumni/Sky-Player/releases/latest)

> [!WARNING]
> Automatically playing music sheets or using simulated keystrokes might violate Thatgamecompany's Terms of Service. Use this tool responsibly and at your own risk.

---

## 🛠️ Quick Start & Installation

### 🚀 Option 1: Standalone Release (Recommended)

1. Go to the [Releases](https://github.com/pumni/Sky-Player/releases) page on GitHub.
2. Download the latest `Sky-Player.zip` package.
3. Extract the ZIP file anywhere on your PC.
4. Launch your **Sky game**, then double-click `Sky-Player.exe` inside the extracted folder to start playing!

### 💻 Option 2: Running from Source

If you prefer running the Python script directly, install Python >= 3.11 and `uv`:

```bash
# Install dependencies
uv sync

# Run the app
uv run python src/main.py
# Or use the quick script: .\play.bat
```

---

## 🎵 How to Use

1. **Open your Sky game** first.
2. **Launch the player**.
3. **Select a song**:
   - Start typing to fuzzy-search by song name.
   - Use the arrow keys to move through results.
   - Press `Enter` to play the selected song.
   - Press `/` to open the command palette.
   - Press `p`, `t`, `f`, or `y` for timing profile, tempo, FPS, or theme.
   - Press `d`, `h`, or `F3` to toggle dry-run, HUD detail, or telemetry.
   - Press `Ctrl+R` to reload songs.
   - Press `q` or `Esc` to quit.

Sky Player uses a beautiful Textual TUI interface for selecting and playing songs. It requires an ANSI-compatible terminal (such as Windows Terminal or the VS Code integrated terminal) to run.

### ➕ Adding More Songs

1. Go to [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Download any song in **JSON**, **skysheet**, or JSON-compatible **txt** format.
3. Save the downloaded file inside the `songs/` directory.
4. Press `Ctrl+R` in the picker to reload the song list.

---

## 🔧 Tuning for Your Machine / Forks

Most users need no extra flags — the defaults are optimised for stock CPython 3.14 on Windows.

For non-standard environments (weak machines, free-threaded `python3.14t` builds, jitter
investigation, maximum-compatibility mode), see **[docs/tuning-presets.md](docs/tuning-presets.md)**
for the full preset table with instructions and telemetry fields to verify.

Run `--doctor` first to check your GIL state, MMCSS availability, and key mapping before
choosing a preset.

---

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

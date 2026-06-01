# 🎵 Sky Children of the Light: PC Precision Music Player

An automatic music player designed for **Sky: Children of the Light** on PC. It reads JSON, skysheet, or JSON-compatible txt song files downloaded from specy/skyMusic and simulates keyboard keypresses in real-time.

> [!WARNING]
> Automatically playing music sheets or using simulated keystrokes might violate Thatgamecompany's Terms of Service. Use this tool responsibly and at your own risk.

---

## 🛠️ Quick Start & Installation

### 🚀 Option 1: Standalone Release (Recommended)

1. Go to the [Releases](https://github.com/pumznguyen/Sky-Player/releases) page on GitHub.
2. Download the latest `Sky-Player.zip` package.
3. Extract the ZIP file anywhere on your PC.
4. Launch your **Sky game**, then double-click `Sky-Player.exe` inside the extracted folder to start playing!

### 💻 Option 2: Running from Source

If you prefer running the Python script directly (requires Python >= 3.11):

```bash
# Install dependencies
pip install -r requirements.txt

# Run the app
python src/main.py
# Or use the quick script: .\play.bat
```
*(Note: If you use `uv`, you can simply run `uv run play`)*

---

## 🎵 How to Use

1. **Open your Sky game** first.
2. **Launch the player**.
3. **Select a song**:
   - Type the song number, name, or a search keyword.
   - Press `/` to open the **Command Palette** to adjust settings (Tempo, FPS, Timing Profiles, etc.).
   - Press `Enter` to play or `Space` for quick play.
   - Type `q` or `Esc` to quit.

### ➕ Adding More Songs

1. Go to [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Download any song in **JSON**, **skysheet**, or JSON-compatible **txt** format.
3. Save the downloaded file inside the `songs/` directory.
4. Type `r` in the player selection screen to instantly load the new songs!

---

## ⚙️ Advanced Settings & CLI

Most settings—including timing profiles, calibration, and FPS adjustments—can be managed effortlessly inside the app using the **Command Palette** (press `/` while in the menu).

If you need to run system diagnostics, generate telemetry logs, or use advanced command-line overrides, you can view all available CLI arguments by running:

```bash
Sky-Player.exe --help
# or
python src/main.py --help
```

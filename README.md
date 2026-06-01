# 🎵 Sky Children of the Light: PC Precision Music Player

An automatic music player designed for **Sky: Children of the Light** on PC. It reads JSON, skysheet, or JSON-compatible txt song files downloaded from specy/skyMusic and simulates keyboard keypresses in real-time.

> [!WARNING]
> Automatically playing music sheets or using simulated keystrokes might violate Thatgamecompany's Terms of Service. Use this tool responsibly and at your own risk.

---

## 🛠️ Quick Start & Installation

Choose one of the options below to get started:

### 🚀 Option 1: Standalone Release (Recommended)

1. Go to the [Releases](https://github.com/pumznguyen/Sky-Player/releases) page on GitHub.
2. Download the latest `Sky-Player.zip` package.
3. Extract the ZIP file anywhere on your PC.
4. Launch your **Sky game**, then double-click `Sky-Player.exe` inside the extracted folder to start playing!

---

### 💻 Option 2: Running from Source (Standard Python)

- **Requirements:** Python >= 3.11

1. **Install Dependencies:**

   ```bash
   pip install -r requirements.txt
   # To install dev dependencies (pytest, etc.)
   pip install pytest prompt-toolkit
   ```

2. **Run the App:**
   ```bash
   python src/main.py
   # Or using the quick script:
   .\play.bat
   ```

---

### ⚡ Option 3: Running from Source (Using `uv`)

1. **Run directly:**
   ```bash
   uv run play
   # Or using the quick script:
   .\play.bat
   ```

---

## 🎵 How to Use

1. **Open your Sky game** first.
2. **Launch the player** using one of the options above.
3. **Select a song**:
   - Type the song number, name, or a search keyword.
   - Press `/` to open the **Command Palette** to adjust settings (Tempo, FPS, Profile, etc.).
   - Press `Enter` to play or `Space` for quick play.
   - Type `q` or `Esc` to quit.

### ➕ Adding More Songs

1. Go to [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Download any song in **JSON**, **skysheet**, or JSON-compatible **txt** format.
3. Save the downloaded file inside the `songs/` directory.
4. Type `r` in the player selection screen to instantly load the new songs!

---

## ⚙️ Precision Timing & Diagnostics

Configure the playback parameters to perfectly fit your system using the following commands:

### 🏥 Clinician Doctor

Diagnose high-precision multimedia timers, key map setups, key depression conflicts, and admin elevations (UIPI):

```bash
# Complete system check
python src/main.py --doctor

# Multimedia Timer diagnostics only
python src/main.py --doctor-timing

# Keyboard mappings & conflict check only
python src/main.py --doctor-input
```

### 🏎️ Timing Profiles & Overrides

Adjust note holds and safety gaps by selecting predefined profiles or overriding constants:

```bash
# Predefined Profiles: balanced (default), local-precise, remote-safe, dense-safe
python src/main.py --song "Song Name" --timing-profile remote-safe

# Manual overrides (in milliseconds)
python src/main.py --song "Song Name" --hold-ms 20 --min-hold-ms 10 --release-gap-ms 4
```

### 🔬 Simulation & CSV Telemetry

Measure hardware timing latency by creating telemetry reports or running mock simulations in memory:

```bash
# Record timing deviation log to logs/ directory
python src/main.py --song "Song Name" --debug-csv

# Dry-run: Simulate precise playback in memory without sending keystrokes
python src/main.py --song 1 --dry-run --debug-csv

# Analyse latest telemetry without changing config.json
python src/main.py --auto-calibrate

# Apply latest telemetry recommendation for the current process only
python src/main.py --apply-calibration

# Persist recommended profile, tempo, FPS, and input lead to config.json
python src/main.py --save-calibration

# Use a specific telemetry summary instead of the latest file in logs/
python src/main.py --save-calibration --calibration-summary logs/run.summary.json
```

---

## ⚙️ CLI Configuration Options

Customize your experience by passing arguments:

```bash
python src/main.py [OPTIONS]
```

<details>
<summary><b>Click to expand all CLI arguments</b></summary>

```text
  --song SONG                Play a song immediately by number, name, keyword, or path
  --list                     List all available songs and exit
  --countdown COUNTDOWN      Seconds to count down before playing (Default: 3)
  --repeat REPEAT            Number of times to repeat the song (Default: 1)
  --no-clear                 Do not clear the console after a song ends

  --doctor                   Run complete clinical diagnostic check
  --doctor-timing            Diagnose high-precision multimedia timers status
  --doctor-input             Diagnose layout configurations & depressed key conflicts

  --timing-profile PROFILE   Select timing profile: balanced, local-precise, remote-safe, dense-safe
  --hold-ms HOLD             Override key hold duration (in milliseconds)
  --min-hold-ms MIN          Override minimum key hold duration (in milliseconds)
  --release-gap-ms GAP       Override release gap (in milliseconds)
  --repeat-release-gap-ms G  Override gap before same-key repeats (in milliseconds)
  --fps FPS                  Game FPS hint for frame-aware timing (30, 60, 120...)
  --frame-align {none,down_only}
                             Optionally snap key-down events to frame boundaries
  --debug-csv                Write CSV telemetry timing logs to logs/
  --dry-run                  Simulate playback in memory without sending keystrokes
  --auto-calibrate           Print recommendations from the latest telemetry summary
  --calibration-summary PATH Use a specific .summary.json, .csv, or logs directory
  --apply-calibration        Apply latest recommendation in memory only
  --save-calibration         Persist latest recommendation to config.json

  --scan-code-mode {physical,mapped}
                             physical: Standard QWERTY layout scan codes (Recommended)
                             mapped: Maps based on active OS keyboard layout
  --sky-process-names NAMES  Comma-separated list of expected Sky game process names
  --allow-title-fallback     Allow window title matching fallback if process checks fail
  --pause-key KEY            Pause/resume hotkey (Default: F8)
  --skip-key KEY             Skip song hotkey (Default: F9)
  --quit-key KEY             Quit program hotkey (Default: Esc)
  --refocus-key KEY          Focus Sky game window hotkey (Default: F6)
  --disable-hotkeys          Disable all hotkeys completely
```

</details>

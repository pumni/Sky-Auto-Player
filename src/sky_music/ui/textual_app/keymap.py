from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class KeyHint:
    key: str
    label: str
    action: str | None = None


@dataclass(frozen=True, slots=True)
class CommandSpec:
    id: str
    key: str
    label: str
    description: str
    group: str


PICKER_HINTS: list[KeyHint] = [
    KeyHint("/", "Commands", "app.open_commands"),
    KeyHint("Enter", "Play", "app.confirm"),
    KeyHint("Esc", "Cancel", "app.cancel"),
    KeyHint("↑↓", "Navigate"),
]

COMMAND_MODAL_HINTS: list[KeyHint] = [
    KeyHint("↑↓", "Move"),
    KeyHint("Enter", "Select"),
    KeyHint("Esc", "Close"),
]

INFO_MODAL_HINTS: list[KeyHint] = [
    KeyHint("↑↓", "Scroll"),
    KeyHint("PgUp/PgDn", "Page"),
    KeyHint("Esc", "Close"),
]


COMMANDS: list[CommandSpec] = [
    CommandSpec("preview", "v", "Song Details", "View selected song details", "View"),
    CommandSpec("hold", "p", "Hold", "Change key hold duration in game frames", "Playback"),
    CommandSpec("tempo", "t", "Adjust Tempo", "Speed up or slow down playback", "Playback"),
    CommandSpec("fps", "f", "FPS Sync", "Synchronize with game frame rate", "Playback"),
    CommandSpec("calibration", "", "Telemetry Playback Recommendation", "Analyze the latest playback telemetry and suggest hold/tempo changes", "Playback"),
    CommandSpec("dry_run", "d", "Toggle Dry-run", "Simulate without sending keys", "Playback"),
    CommandSpec("calibrate_latency", "c", "Host Hold Margin Calibration", "Measure Rust + SendInput completion-hold shrink", "Playback"),
    CommandSpec("hud", "h", "Toggle HUD", "Show/hide TUI HUD and Debug panel", "Interface"),
    CommandSpec("telemetry", "F3", "Toggle Telemetry", "Enable/disable CSV logging", "Interface"),
    CommandSpec("reload", "Ctrl+R", "Reload Songs", "Refresh songs directory", "Library"),
    CommandSpec("theme", "y", "Change Theme", "Switch UI color scheme", "Interface"),
    CommandSpec("help", "?", "Help", "Show available picker commands", "System"),
    CommandSpec("update_settings", "", "Update Settings", "Toggle automatic version checks", "System"),
    CommandSpec("update", "", "Check for Update", "Check GitHub for a newer release", "System"),
]

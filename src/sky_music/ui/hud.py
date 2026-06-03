import shutil
import os
import time
import re
from sky_music.infrastructure.hotkeys import PlaybackControls

PLAYBACK_FINISHED = "finished"
PLAYBACK_SKIPPED = "skipped"
PLAYBACK_QUIT = "quit"
PLAYBACK_POLL_SECONDS = 0.025
PROGRESS_RENDER_INTERVAL_SECONDS = 0.10

def format_duration(seconds: float) -> str:
    seconds = max(0, int(seconds))
    minutes, sec = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    if hours:
        return f"{hours}:{minutes:02}:{sec:02}"
    return f"{minutes}:{sec:02}"

class ProgressRenderer:
    def __init__(
        self,
        controls: PlaybackControls | None = None,
        verbose: bool = False,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
    ) -> None:
        self.controls = controls
        self.verbose = verbose
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.last_render_at: float = 0.0
        
        # Live timing counters updated by PlaybackEngine
        self.late_2ms: int = 0
        self.late_5ms: int = 0
        self.late_10ms: int = 0
        self.max_lateness_us: int = 0
        
        self.run_id: str = ""
        self.last_lines_printed: int = 0
        self._initialized: bool = False

    def update_counters(self, lateness_us: int) -> None:
        """Called by PlaybackEngine after each key action to update live timing counters."""
        if lateness_us > 10000:
            self.late_10ms += 1
            self.late_5ms += 1
            self.late_2ms += 1
        elif lateness_us > 5000:
            self.late_5ms += 1
            self.late_2ms += 1
        elif lateness_us > 2000:
            self.late_2ms += 1
        if lateness_us > self.max_lateness_us:
            self.max_lateness_us = lateness_us

    def render(self, current: float, total: float, song_name: str, status: str = "playing", force: bool = False) -> None:
        now = time.perf_counter()
        if not force and now - self.last_render_at < PROGRESS_RENDER_INTERVAL_SECONDS:
            return

        self.last_render_at = now
        
        if not self.run_id:
            self.run_id = time.strftime('%Y%m%d-%H%M%S')
            
        terminal_width = shutil.get_terminal_size((100, 20)).columns
        width = min(80, terminal_width)
        
        # ANSI Colors
        ANSI_RESET = "\033[0m"
        ANSI_BOLD = "\033[1m"
        ANSI_CYAN = "\033[96m"
        ANSI_MAGENTA = "\033[95m"
        ANSI_GREEN = "\033[92m"
        ANSI_YELLOW = "\033[93m"
        ANSI_RED = "\033[91m"
        ANSI_GRAY = "\033[90m"
        
        def len_ansi(s: str) -> int:
            return len(re.sub(r'\033\[[0-9;]*m', '', s))
            
        def pad_ansi(s: str, w: int) -> str:
            cur_len = len_ansi(s)
            if cur_len < w:
                return s + " " * (w - cur_len)
            return s
            
        def ansi_box(title: str, lines: list[str], border_color: str = ANSI_CYAN) -> list[str]:
            top_left = "╭"
            top_right = "╮"
            bottom_left = "╰"
            bottom_right = "╯"
            horiz = "─"
            vert = "│"
            
            title_part = f"{horiz} {title} "
            top_line = f"{border_color}{top_left}{title_part}{horiz * (width - len(title_part) - 2)}{top_right}{ANSI_RESET}"
            bottom_line = f"{border_color}{bottom_left}{horiz * (width - 2)}{bottom_right}{ANSI_RESET}"
            
            out = [top_line]
            for line in lines:
                padded = pad_ansi(line, width - 4)
                out.append(f"{border_color}{vert}{ANSI_RESET} {padded} {border_color}{vert}{ANSI_RESET}")
            out.append(bottom_line)
            return out

        # 1. Header Box
        if status == "playing":
            header_label = "Playing"
        elif status == "paused":
            header_label = "Paused"
        elif status == "focus_lost":
            header_label = "Focus Lost"
        elif status == "waiting_for_focus":
            header_label = "Waiting for Focus"
        elif status == "refocus":
            header_label = "Refocusing"
        elif status == "panic":
            header_label = "Panic Release"
        elif status == "done":
            header_label = "Done"
        else:
            header_label = status.replace("_", " ").title()
            
        header_line = f"{header_label} │ profile {ANSI_CYAN}{self.profile_name}{ANSI_RESET} │ tempo {ANSI_CYAN}{self.tempo_scale:.2f}x{ANSI_RESET} │ run {ANSI_CYAN}{self.run_id}{ANSI_RESET}"
        header_box = ansi_box("SKY MUSIC HELPER", [header_line], border_color=ANSI_CYAN)
        
        # 2. Song Box
        total_time_str = format_duration(total)
        current_time_str = format_duration(current)
        time_text = f"{current_time_str} / {total_time_str}"
        
        bar_width = width - len(time_text) - 8
        fraction = current / max(total, 0.001)
        filled = min(bar_width, int(round(fraction * bar_width)))
        bar = f"{ANSI_GREEN}█{ANSI_RESET}" * filled + f"{ANSI_GRAY}░{ANSI_RESET}" * (bar_width - filled)
        
        song_lines = [
            f"{ANSI_BOLD}{song_name}{ANSI_RESET}",
            f"{bar}  {time_text}"
        ]
        song_box = ansi_box("Song", song_lines, border_color=ANSI_CYAN)
        
        # 3. Retrieve backend health dynamically
        health = getattr(self, "backend", None)
        active_keys = 0
        failed_releases = 0
        if health is not None and hasattr(health, "get_health"):
            h = health.get_health()
            active_keys = h.active_count
            failed_releases = h.failed_release_count
            
        backend_status = f"{ANSI_RED}stuck keys: {failed_releases}{ANSI_RESET}" if failed_releases > 0 else f"{ANSI_GREEN}healthy{ANSI_RESET}"
        
        # 4. Status Box
        if status == "waiting_for_focus":
            status_line = f"{ANSI_YELLOW}Playback has not started yet. Bring Sky window to foreground.{ANSI_RESET}"
            if self.controls is not None and self.controls.enabled:
                controls_line = (
                    f"{ANSI_BOLD}{self.controls.refocus.display}{ANSI_RESET} refocus │ "
                    f"{ANSI_BOLD}{self.controls.quit.display}{ANSI_RESET} quit │ "
                    f"{ANSI_BOLD}D{ANSI_RESET} dry-run │ "
                    f"{ANSI_BOLD}{self.controls.panic.display}{ANSI_RESET} panic"
                )
            else:
                controls_line = "hotkeys disabled"
            status_color = ANSI_YELLOW
            
        elif status == "focus_lost":
            status_line = f"{ANSI_RED}Playback is paused and tracked keys were released.{ANSI_RESET}"
            if self.controls is not None and self.controls.enabled:
                controls_line = (
                    f"{ANSI_BOLD}{self.controls.refocus.display}{ANSI_RESET} refocus │ "
                    f"{ANSI_BOLD}{self.controls.quit.display}{ANSI_RESET} quit │ "
                    f"{ANSI_BOLD}{self.controls.panic.display}{ANSI_RESET} panic"
                )
            else:
                controls_line = "hotkeys disabled"
            status_color = ANSI_RED
            
        elif status == "paused":
            status_line = f"{ANSI_YELLOW}Playback is paused and tracked keys were released.{ANSI_RESET}"
            if self.controls is not None and self.controls.enabled:
                controls_line = (
                    f"{ANSI_BOLD}{self.controls.pause.display}{ANSI_RESET} resume │ "
                    f"{ANSI_BOLD}{self.controls.skip.display}{ANSI_RESET} skip │ "
                    f"{ANSI_BOLD}{self.controls.quit.display}{ANSI_RESET} quit │ "
                    f"{ANSI_BOLD}{self.controls.refocus.display}{ANSI_RESET} refocus │ "
                    f"{ANSI_BOLD}{self.controls.panic.display}{ANSI_RESET} panic"
                )
            else:
                controls_line = "hotkeys disabled"
            status_color = ANSI_YELLOW
            
        else: # playing, done, refocus, panic
            if self.verbose:
                status_line = f"backend {backend_status} │ late >2ms:{self.late_2ms}  >5ms:{self.late_5ms}  >10ms:{self.late_10ms} │ active keys: {active_keys}"
            else:
                status_line = f"backend {backend_status} │ late >5ms: {self.late_5ms} │ active keys: {active_keys}"
                
            if self.controls is not None and self.controls.enabled:
                controls_line = (
                    f"{ANSI_BOLD}{self.controls.pause.display}{ANSI_RESET} pause │ "
                    f"{ANSI_BOLD}{self.controls.skip.display}{ANSI_RESET} skip │ "
                    f"{ANSI_BOLD}{self.controls.quit.display}{ANSI_RESET} quit │ "
                    f"{ANSI_BOLD}{self.controls.refocus.display}{ANSI_RESET} refocus │ "
                    f"{ANSI_BOLD}{self.controls.panic.display}{ANSI_RESET} panic"
                )
            else:
                controls_line = "hotkeys disabled"
            status_color = ANSI_MAGENTA
            
        lines = [status_line, controls_line]
        if self.verbose and getattr(self, "active_policy", None) is not None:
            pol = self.active_policy
            frame_label = f"{pol.frame_us}us" if pol.frame_us > 0 else "N/A"
            fps_label = f"{pol.fps}fps" if pol.fps > 0 else "N/A"

            cycle_info = f"cycle: {int(pol.min_hold_us) + int(pol.repeat_release_gap_us)}us"
            hold_info = f"hold/min/gap: {pol.hold_us}/{pol.min_hold_us}/{pol.repeat_release_gap_us}us"
            
            timing_line = f"{ANSI_GRAY}Timing: {fps_label} ({frame_label}) │ {cycle_info} │ {hold_info}{ANSI_RESET}"
            lines.insert(1, timing_line)

        status_box = ansi_box("Status", lines, border_color=status_color)
        
        hud_lines = header_box + [""] + song_box + [""] + status_box
        
        if self._initialized and self.last_lines_printed > 0:
            print(f"\033[{self.last_lines_printed}A", end="", flush=True)
            
        self._initialized = True
        self.last_lines_printed = len(hud_lines)
        
        output = "\n".join(hud_lines) + "\n"
        print(output, end="", flush=True)

    def finish(self, message: str = "") -> None:
        """Erase the live HUD from the terminal. The caller is responsible for
        redrawing (e.g. picker loop) — no status message is printed here."""
        if self._initialized and self.last_lines_printed > 0:
            print(f"\033[{self.last_lines_printed}A", end="", flush=True)
            for _ in range(self.last_lines_printed):
                print("\r\033[K")
            print(f"\033[{self.last_lines_printed}A", end="", flush=True)
        self._initialized = False
        self.last_lines_printed = 0

# Compatibility Alias
PlaybackHudRenderer = ProgressRenderer

def clear_terminal() -> None:
    os.system('cls' if os.name == 'nt' else 'clear')

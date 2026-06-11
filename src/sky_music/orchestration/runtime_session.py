from dataclasses import dataclass
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.config import AppConfig, RtPriorityMode

@dataclass
class PlaybackOverrides:
    dry_run: bool = False
    profile: str | None = None
    tempo: float | None = None
    fps: int | None = None
    dispatch_lead_us: int = 0


@dataclass
class RuntimeSessionState:
    session: PlaybackSessionContext | None = None
    timing_policy: object | None = None
    sleep_policy: object | None = None
    scan_code_mode: str = "physical"
    telemetry_csv_enabled: bool = False
    dry_run: bool = False
    tempo_scale: float = 1.0
    timing_profile_name: str = "balanced"
    verbose_hud: bool = False
    use_dispatch_thread: bool = True
    enable_timer_guard: bool = True
    enable_waitable_timer: bool = True
    enable_gc_pause: bool = True
    enable_switch_interval_tuning: bool = True
    # Graduated defaults (2026-06-11): adaptive lead/spin, event-driven waits, and the MMCSS
    # priority ladder all ship ON; --no-* CLI flags are the per-feature kill switches.
    enable_adaptive_lead: bool = True
    enable_adaptive_spin: bool = True
    enable_event_wait: bool = True
    enable_epoch_rebase: bool = True
    rt_priority_mode: RtPriorityMode = "auto"
    check_input_path: bool = False

    def apply_session(self, session: PlaybackSessionContext, cfg: AppConfig, *, spin_threshold_us: int | None = None) -> None:
        self.session = session
        self.timing_policy = session.resolve_effective_policy(cfg)
        self.sleep_policy = session.resolve_sleep_policy(cfg, spin_threshold_us=spin_threshold_us)
        self.scan_code_mode = session.scan_code_mode
        self.tempo_scale = session.tempo_scale
        self.timing_profile_name = session.display_profile_label()


RUNTIME_STATE = RuntimeSessionState()

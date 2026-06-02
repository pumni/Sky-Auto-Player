import csv
import json
from pathlib import Path
import sys
import time
import random
from sky_music.infrastructure.backend import BackendHealth

class TelemetryLogger:
    """Records precise microsecond timing metrics into clean CSV and companion summary JSON files for calibration."""
    def __init__(
        self,
        song_name: str,
        enabled: bool = False,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        run_id: str | None = None,
        fps: int | None = None
    ):
        self.song_name = song_name
        self.enabled = enabled
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.fps = fps
        self.records = []
        self.log_filepath = None
        self.backend_health: BackendHealth | None = None
        self.release_outcome = None
        self.schedule_summary: dict | None = None
        
        # Unique run ID generation
        if run_id is None:
            self.run_id = f"{time.strftime('%Y%m%d-%H%M%S')}-{random.randint(1000, 9999)}"
        else:
            self.run_id = run_id
        
        if self.enabled:
            logs_dir = Path("logs")
            logs_dir.mkdir(parents=True, exist_ok=True)
            self.log_filepath = logs_dir / f"playback_telemetry_{self.run_id}.csv"
            
    def record(
        self,
        event_index: int,
        kind: str,
        scheduled_us: int,
        actual_us: int,
        lateness_us: int,
        send_duration_us: int,
        scan_codes: tuple[int, ...],
        reason: str
    ) -> None:
        if not self.enabled:
            return
            
        scan_codes_str = ";".join(str(sc) for sc in scan_codes)
        self.records.append({
            "song": self.song_name,
            "event_index": event_index,
            "kind": kind,
            "scheduled_us": scheduled_us,
            "actual_us": actual_us,
            "lateness_us": lateness_us,
            "send_duration_us": send_duration_us,
            "scan_codes": scan_codes_str,
            "reason": reason
        })
        
    def record_backend_health(self, health: BackendHealth) -> None:
        """Stores the backend health state at the end of playback."""
        self.backend_health = health

    def record_release_outcome(self, outcome) -> None:
        """Stores the final release_all outcome at the end of playback."""
        self.release_outcome = outcome

    def record_schedule_metadata(self, metadata) -> None:
        """Stores scheduler stress metrics for later calibration."""
        self.schedule_summary = {
            "compressed_holds": int(getattr(metadata, "compressed_holds", 0)),
            "impossible_same_key_repeats": int(getattr(metadata, "impossible_same_key_repeats", 0)),
            "risky_same_key_repeats": int(getattr(metadata, "risky_same_key_repeats", 0)),
            "max_polyphony": int(getattr(metadata, "max_polyphony", 0)),
            "note_count": int(getattr(metadata, "note_count", 0)),
            "shortest_same_key_interval_us": getattr(metadata, "shortest_same_key_interval_us", None),
            "frame_us": getattr(metadata, "frame_us", None),
            "fps": getattr(metadata, "fps", None),
            "base_input_lead_us": getattr(metadata, "base_input_lead_us", None),
            "runtime_input_lead_us": getattr(metadata, "runtime_input_lead_us", None),
            "phase_compensated": getattr(metadata, "phase_compensated", None),
            "chord_merge_window_us": getattr(metadata, "chord_merge_window_us", None),
            "frame_align": getattr(metadata, "frame_align", None),
        }

    def get_summary(self) -> dict | None:
        """Compute and return the stats dict in-memory (no file I/O).

        Returns None if there are no records (e.g. telemetry disabled and
        no events recorded).  Callers should guard against None.
        """
        if not self.records:
            return None

        latenesses = [r["lateness_us"] for r in self.records]
        send_durations = [r["send_duration_us"] for r in self.records]

        over_2ms = sum(1 for lat in latenesses if lat > 2000)
        over_5ms = sum(1 for lat in latenesses if lat > 5000)
        over_10ms = sum(1 for lat in latenesses if lat > 10000)

        hold_durations: list[int] = []
        active_downs: dict[int, int] = {}
        for r in self.records:
            codes = [int(sc) for sc in r["scan_codes"].split(";") if sc]
            if r["kind"] == "down":
                for sc in codes:
                    active_downs[sc] = r["actual_us"]
            elif r["kind"] == "up":
                for sc in codes:
                    if sc in active_downs:
                        hold_durations.append(r["actual_us"] - active_downs[sc])
                        del active_downs[sc]

        def _pct(values: list[int], pct: float) -> float:
            if not values:
                return 0.0
            s = sorted(values)
            idx = int(round(pct * (len(s) - 1)))
            return float(s[idx])

        def _stats(values: list[int], thresholds: bool = False) -> dict:
            if not values:
                base: dict = {"p50_us": 0.0, "p95_us": 0.0, "p99_us": 0.0, "max_us": 0.0, "avg_us": 0.0}
                if thresholds:
                    base.update({"over_2ms": 0, "over_5ms": 0, "over_10ms": 0})
                return base
            res = {
                "p50_us": _pct(values, 0.50),
                "p95_us": _pct(values, 0.95),
                "p99_us": _pct(values, 0.99),
                "max_us": float(max(values)),
                "avg_us": float(sum(values) / len(values)),
            }
            if thresholds:
                res.update({"over_2ms": over_2ms, "over_5ms": over_5ms, "over_10ms": over_10ms})
            return res

        backend_info: dict = {"panic_release_failures": 0, "failed_release_keys_final": []}
        if self.backend_health is not None:
            backend_info["panic_release_failures"] = self.backend_health.failed_release_count
            
        if self.release_outcome is not None:
            backend_info["release_attempted"] = self.release_outcome.attempted
            backend_info["release_success"] = self.release_outcome.released_successfully
            backend_info["release_stuck_keys"] = self.release_outcome.stuck_keys
            backend_info["release_inconclusive"] = self.release_outcome.verification_inconclusive

        summary = {
            "run_id": self.run_id,
            "song": self.song_name,
            "profile": self.profile_name,
            "fps": self.fps,
            "tempo_scale": self.tempo_scale,
            "total_events": len(self.records),
            "lateness_us": _stats(latenesses, thresholds=True),
            "send_duration_us": _stats(send_durations),
            "note_hold_duration_us": _stats(hold_durations),
            "backend": backend_info,
        }
        if self.schedule_summary is not None:
            summary["schedule"] = self.schedule_summary
        return summary
        
    def save(self) -> None:
        if not self.enabled or not self.log_filepath or not self.records:
            return

        try:
            # 1. Save standard raw CSV records
            fields = ["song", "event_index", "kind", "scheduled_us", "actual_us", "lateness_us", "send_duration_us", "scan_codes", "reason"]
            with self.log_filepath.open("w", newline="", encoding="utf-8") as f:
                writer = csv.DictWriter(f, fieldnames=fields)
                writer.writeheader()
                writer.writerows(self.records)

            # 2. Reuse get_summary() — augment with timestamp for the persisted JSON
            summary = self.get_summary()
            if summary is None:
                return
            summary["timestamp"] = time.strftime('%Y-%m-%d %H:%M:%S')

            # 3. Save companion summary JSON
            summary_path = self.log_filepath.with_suffix(".summary.json")
            with summary_path.open("w", encoding="utf-8") as f:
                json.dump(summary, f, indent=2)

        except Exception as e:
            sys.stderr.write(f"[telemetry] failed to save metrics: {e}\n")

def inspect_telemetry_report(target_path: str, recommend: bool = False) -> None:
    """Load and format a timing performance report from companion summary JSON telemetry files."""
    path = Path(target_path)
    summary_files = []
    
    if path.is_file():
        if path.suffix == ".json":
            summary_files.append(path)
        elif path.suffix == ".csv":
            summary_files.append(path.with_suffix(".summary.json"))
    elif path.is_dir():
        summary_files = list(path.glob("*.summary.json"))
        
    summary_files = [f for f in summary_files if f.exists()]
    if not summary_files:
        print(f"No valid telemetry summary files (.summary.json) found at {target_path}")
        return
        
    print(f"\n==================================================")
    print(f" AGGREGATE TELEMETRY TIMING REPORT ({len(summary_files)} run(s))")
    print(f"==================================================")
    
    for f in summary_files:
        try:
            with f.open("r", encoding="utf-8") as file:
                data = json.load(file)
                
            print(f"\nPlayback: {data.get('song', 'Unknown')} at {data.get('timestamp', 'Unknown')} [Run ID: {data.get('run_id', 'N/A')}]")
            print(f"  Profile: {data.get('profile', 'balanced')} | Tempo Scale: {data.get('tempo_scale', 1.0)}")
            print(f"  Total Event Count: {data.get('total_events', 0)}")
            
            lat = data.get("lateness_us", {})
            print(f"  Loop Lateness:")
            print(f"    * Average: {lat.get('avg_us', 0.0):.1f} us ({lat.get('avg_us', 0.0)/1000:.3f} ms)")
            print(f"    * Median (p50): {lat.get('p50_us', 0.0):.1f} us")
            print(f"    * 95th Percentile (p95): {lat.get('p95_us', 0.0):.1f} us")
            print(f"    * 99th Percentile (p99): {lat.get('p99_us', 0.0):.1f} us")
            print(f"    * Maximum: {lat.get('max_us', 0.0):.1f} us")
            print(f"    * Lateness Counts: >2ms={lat.get('over_2ms', 0)}, >5ms={lat.get('over_5ms', 0)}, >10ms={lat.get('over_10ms', 0)}")
            
            dur = data.get("send_duration_us", {})
            print(f"  SendInput Execution Duration:")
            print(f"    * Average: {dur.get('avg_us', 0.0):.1f} us")
            print(f"    * p95: {dur.get('p95_us', 0.0):.1f} us")
            print(f"    * p99: {dur.get('p99_us', 0.0):.1f} us")
            
            hold = data.get("note_hold_duration_us", {})
            if hold:
                print(f"  Note Hold Durations:")
                print(f"    * Average: {hold.get('avg_us', 0.0):.1f} us ({hold.get('avg_us', 0.0)/1000:.1f} ms)")
                print(f"    * p50: {hold.get('p50_us', 0.0):.1f} us")
                
            backend = data.get("backend", {})
            if backend.get("panic_release_failures", 0) > 0:
                print(f"  [warning] Backend panic release failures count: {backend.get('panic_release_failures')}")
                
            # Perform calibration recommendation if requested
            if recommend:
                from sky_music.orchestration.calibration import calibrate_profile, calibration_input_from_summary
                inp = calibration_input_from_summary(data)
                rec = calibrate_profile(inp)
                
                print(f"\n  Calibration Recommendation:")
                print(f"    * Suggested Profile : {rec.profile_name}")
                print(f"    * Suggested Tempo   : {rec.tempo_scale:.2f}x")
                print(f"    * Input Lead (us)   : {rec.input_lead_us} ({rec.input_lead_us/1000:.1f} ms)")
                print(f"    * Hold Duration (us): {rec.hold_us} ({rec.hold_us/1000:.1f} ms)")
                print(f"    * Severity Level    : {rec.severity.upper()}")
                print(f"    * Reason            : {rec.reason}")
        except Exception as e:
            print(f"  [error] Failed to read summary file {f.name}: {e}")
            
    print(f"\n==================================================")

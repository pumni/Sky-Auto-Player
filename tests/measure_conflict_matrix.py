"""Measure dropped notes (dropped_conflict) by (profile x fps x send_duration).

Runs the REAL PlaybackEngine on a virtual clock (FakeClock + TimedBackend) without
real-time waits, so the entire corpus is scanned in a few seconds. Every dropped-note
count comes directly from RuntimeDispatchCoordinator.split_down_intents -> dropped_conflict.

Usage:
    python tests/measure_conflict_matrix.py            # full corpus, summary table
    python tests/measure_conflict_matrix.py "blue"     # filter by name (substring)
"""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from sky_music.domain import Song  # noqa: E402
from sky_music.domain.parser import SongParseError, parse_song_file  # noqa: E402
from sky_music.domain.scheduler import build_key_actions  # noqa: E402
from sky_music.domain.scheduler_types import FrameTimingPolicy  # noqa: E402
from sky_music.orchestration.runtime_dispatch import (  # noqa: E402
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)


def _dispatch_down(coord: RuntimeDispatchCoordinator, batch, now: int, send_us: int) -> int:
    """Mirror engine._dispatch_down_batch (no late-drop: threshold is None in prod)."""
    playable, _conflicts = coord.split_down_intents(batch.intents)  # marks dropped_conflict
    if not playable:
        return now
    started = now
    now += send_us  # backend SendInput latency -> completion
    sent = tuple(int(i.scan_code) for i in playable)
    coord.activate_sent_downs(
        playable, sent, dispatch_started_us=started, dispatch_completed_us=now
    )
    return now


def _dispatch_releases(coord: RuntimeDispatchCoordinator, releases, now: int, send_us: int) -> int:
    if not releases:
        return now
    now += send_us
    sent = tuple(int(r.scan_code) for r in releases)
    coord.complete_releases(releases, sent, ())
    return now


def simulate(coord: RuntimeDispatchCoordinator, send_us: int) -> None:
    """Replay the coordinator exactly like engine._run_dispatch/_drain_due, virtual clock."""
    now = 0
    while not coord.is_finished():
        deadline = coord.next_deadline_us()
        if deadline is None:
            break
        now = max(now, deadline)  # wait until the deadline; clock never moves backward

        pending = coord.pop_due_pending(now)
        now = _dispatch_releases(coord, pending, now, send_us)

        for batch in coord.pop_due_authored(now):
            if batch.kind == "up":
                coord.request_releases(batch.intents)
                newly_due = coord.pop_due_pending(now)
                now = _dispatch_releases(coord, newly_due, now, send_us)
            else:
                now = _dispatch_down(coord, batch, now, send_us)


def run_one(song: Song, profile: str, fps: int, send_duration_us: int) -> dict:
    policy = FrameTimingPolicy.from_profile_name(profile, fps=fps)
    sched = build_key_actions(song, policy=policy)
    schedule = compile_runtime_intents(sched.actions)
    coord = RuntimeDispatchCoordinator(schedule, int(policy.min_hold_us))
    simulate(coord, send_duration_us)

    counts = coord.generation_status_counts()
    intended = schedule.generation_count  # one generation per authored key-down
    dropped = counts["dropped_conflict"]
    return {
        "min_hold_us": int(policy.min_hold_us),
        "intended": intended,
        "sent": intended - dropped - counts["dropped_backend"],
        "dropped_conflict": dropped,
        "sched_impossible": sched.impossible_same_key_repeats,
        "dedup_notes": sched.deduplicated_note_count,
    }


def load_songs(filter_sub: str | None) -> list[Song]:
    songs_dir = ROOT / "songs"
    out: list[Song] = []
    for path in sorted(songs_dir.glob("*.json")):
        if filter_sub and filter_sub.lower() not in path.name.lower():
            continue
        try:
            song = parse_song_file(path)
        except (SongParseError, Exception):
            continue
        if song.notes:
            out.append(song)
    return out


PROFILES = ("local_precise", "balanced", "audience_safe")
FPS_LEVELS = (30, 60, 90, 120, 144, 240)
SEND_DURATIONS = (0, 250, 500)


def main() -> None:
    filter_sub = sys.argv[1] if len(sys.argv) > 1 else None
    songs = load_songs(filter_sub)
    print(f"Loaded {len(songs)} song(s)"
          + (f" matching {filter_sub!r}" if filter_sub else "") + "\n")

    # Aggregate matrix: profile x fps x send_duration -> totals across corpus
    print("=" * 96)
    print("AGGREGATE over corpus — total dropped_conflict downs (and % of intended)")
    print("=" * 96)
    header = f"{'profile':<15}{'fps':>5}{'min_hold_us':>12}"
    for sd in SEND_DURATIONS:
        header += f"{'sd=' + str(sd):>16}"
    print(header)
    print("-" * 96)

    total_intended_ref = None
    for profile in PROFILES:
        for fps in FPS_LEVELS:
            row = f"{profile:<15}{fps:>5}"
            min_hold_shown = None
            cells = []
            for sd in SEND_DURATIONS:
                tot_drop = 0
                tot_intended = 0
                for song in songs:
                    r = run_one(song, profile, fps, sd)
                    tot_drop += r["dropped_conflict"]
                    tot_intended += r["intended"]
                    min_hold_shown = r["min_hold_us"]
                pct = (100.0 * tot_drop / tot_intended) if tot_intended else 0.0
                cells.append(f"{tot_drop} ({pct:.2f}%)")
                total_intended_ref = tot_intended
            row += f"{min_hold_shown:>12}"
            for c in cells:
                row += f"{c:>16}"
            print(row)
        print("-" * 96)
    print(f"\n(total intended same-corpus downs ~= {total_intended_ref})")

    # Split real vs synthetic probes (TEST_* are deliberately sub-frame / infeasible).
    real = [s for s in songs if not s.name.startswith("TEST_")]
    synth = [s for s in songs if s.name.startswith("TEST_")]
    print(f"\nCorpus split: {len(real)} real song(s), {len(synth)} synthetic TEST_ probe(s)")

    # Aggregate over REAL songs only — strips out the deliberately-broken probes.
    print("\n" + "=" * 96)
    print("REAL SONGS ONLY — total dropped_conflict (sd=0 ideal sender / sd=250 realistic latency)")
    print("=" * 96)
    print(f"{'profile':<15}{'fps':>5}{'min_hold_us':>12}{'drop sd=0':>14}{'drop sd=250':>14}{'#songs hit':>12}")
    print("-" * 96)
    for profile in PROFILES:
        for fps in FPS_LEVELS:
            d0 = d250 = hit = 0
            mh = None
            for song in real:
                r0 = run_one(song, profile, fps, 0)
                r250 = run_one(song, profile, fps, 250)
                d0 += r0["dropped_conflict"]
                d250 += r250["dropped_conflict"]
                mh = r0["min_hold_us"]
                if r250["dropped_conflict"] > 0:
                    hit += 1
            print(f"{profile:<15}{fps:>5}{mh:>12}{d0:>14}{d250:>14}{hit:>12}")
        print("-" * 96)

    # Which real songs actually drop, and at which fps (local_precise, sd=250).
    print("\n" + "=" * 96)
    print("REAL songs that drop ANY note (local_precise, sd=250) — by fps")
    print("=" * 96)
    print(f"{'song':<40}{'notes':>7}{'minInt_ms':>10}" + "".join(f"{('@'+str(f)):>7}" for f in FPS_LEVELS))
    print("-" * 96)
    any_real = 0
    for song in real:
        drops = {f: run_one(song, "local_precise", f, 250)["dropped_conflict"] for f in FPS_LEVELS}
        if max(drops.values()) == 0:
            continue
        any_real += 1
        # shortest same-key interval (ms) straight from the scheduler metadata
        meta = build_key_actions(song, policy=FrameTimingPolicy.local_precise(fps=144))
        min_int = meta.shortest_same_key_interval_us
        min_int_ms = f"{min_int/1000:.1f}" if min_int is not None else "-"
        print(f"{song.name[:38]:<40}{meta.deduplicated_note_count:>7}{min_int_ms:>10}"
              + "".join(f"{drops[f]:>7}" for f in FPS_LEVELS))
    print(f"\n({any_real} real song(s) drop at least one note in some fps at local_precise sd=250)")

    # ---------------------------------------------------------------------------------------------
    # GAME-SIDE PROXY (sender cannot observe this): for same-key repeats, the game needs to SEE a
    # release gap to re-trigger. The sender emits observed_hold ~= min_hold, so the gap the game
    # gets is ~ (interval - min_hold). Count same-key transitions whose gap falls under candidate
    # game re-trigger walls. min_hold grows as fps drops / profile multiplier rises, so the same
    # song crosses the wall purely by changing profile/fps.
    # ---------------------------------------------------------------------------------------------
    def same_key_intervals_us(song: Song) -> list[int]:
        from sky_music.layouts import SKY_15_KEY_PROFILE, DefaultNoteResolver
        resolver = DefaultNoteResolver(SKY_15_KEY_PROFILE)
        last_by_sc: dict[int, int] = {}
        intervals: list[int] = []
        # dedup identical (time, key) like the scheduler does
        seen: set[tuple[int, int]] = set()
        for note in sorted(song.notes, key=lambda n: n.time_ms):
            k = note.key
            if k[:1] in "123" and k.startswith(("1Key", "2Key", "3Key")):
                k = "Key" + k[4:]
            try:
                sc = resolver.resolve_scan_code(k, "physical")  # type: ignore[arg-type]
            except Exception:
                continue
            if sc <= 0:
                continue
            t = round(note.time_ms * 1000)
            if (t, sc) in seen:
                continue
            seen.add((t, sc))
            if sc in last_by_sc:
                intervals.append(t - last_by_sc[sc])
            last_by_sc[sc] = t
        return intervals

    real_intervals = {s.name: same_key_intervals_us(s) for s in real}
    total_transitions = sum(len(v) for v in real_intervals.values())

    print("\n" + "=" * 96)
    print("GAME-SIDE RISK PROXY — same-key transitions whose release gap (interval - min_hold)")
    print("falls under a candidate game re-trigger wall. Real songs only. (sender shows 0 drops!)")
    print("=" * 96)
    walls = (0, 8_000, 12_000, 16_000)
    print(f"total same-key transitions across real corpus: {total_transitions}\n")
    print(f"{'profile':<15}{'fps':>5}{'min_hold_us':>12}"
          + "".join(f"{('gap<' + str(w//1000) + 'ms'):>14}" for w in walls))
    print("-" * 96)
    for profile in PROFILES:
        for fps in FPS_LEVELS:
            mh = int(FrameTimingPolicy.from_profile_name(profile, fps=fps).min_hold_us)
            counts = []
            for w in walls:
                c = 0
                for ivs in real_intervals.values():
                    for iv in ivs:
                        if iv - mh < w:
                            c += 1
                counts.append(c)
            print(f"{profile:<15}{fps:>5}{mh:>12}"
                  + "".join(f"{c:>14}" for c in counts))
        print("-" * 96)


if __name__ == "__main__":
    main()

from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass
from pathlib import Path

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from sky_music.config import AppConfig  # noqa: E402
from sky_music.domain.parser import parse_song_file  # noqa: E402
from sky_music.domain.scheduler import build_key_actions  # noqa: E402
from sky_music.domain.scheduler_types import FrameTimingPolicy, KeyAction  # noqa: E402
from sky_music.domain.session_context import PlaybackSessionContext  # noqa: E402
from sky_music.layouts import DefaultNoteResolver, SKY_15_KEY_PROFILE  # noqa: E402


SUPPORTED_SUFFIXES = {".json", ".skysheet"}


@dataclass(frozen=True, slots=True)
class NoteDraft:
    down_us: int
    scan_code: int


@dataclass(frozen=True, slots=True)
class SongRepeatStats:
    path: Path
    note_count: int
    peak_notes_per_second: int
    repeat_candidates: int
    binding_count: int
    positive_binding_count: int
    compressible_binding_count: int
    under_cycle_count: int
    positive_under_cycle_count: int
    zero_interval_count: int
    compressed_holds: int
    impossible_repeats: int
    duplicate_note_count: int
    min_same_key_up_gap_us: int | None
    min_same_key_interval_us: int | None
    min_positive_same_key_interval_us: int | None
    actual_repeat_gaps_us: tuple[int, ...]


@dataclass(frozen=True, slots=True)
class CorpusRepeatStats:
    songs: tuple[SongRepeatStats, ...]
    failed_files: tuple[tuple[Path, str], ...]


def fmt_us(value: int | None) -> str:
    if value is None:
        return "-"
    if abs(value) >= 1000:
        return f"{value / 1000:.3f}ms"
    return f"{value}us"


def fmt_pct(numerator: int, denominator: int) -> str:
    if denominator == 0:
        return "0.000%"
    return f"{100 * numerator / denominator:.3f}%"


def collect_song_paths(songs_dir: Path) -> tuple[Path, ...]:
    return tuple(
        sorted(
            path
            for path in songs_dir.iterdir()
            if path.is_file() and path.suffix.lower() in SUPPORTED_SUFFIXES
        )
    )


def resolve_drafts(song_path: Path, tempo_scale: float) -> tuple[NoteDraft, ...]:
    song = parse_song_file(song_path, SKY_15_KEY_PROFILE)
    resolver = DefaultNoteResolver(SKY_15_KEY_PROFILE)
    drafts: list[NoteDraft] = []
    for note in song.notes:
        key = note.key
        if key.startswith("1Key") or key.startswith("2Key") or key.startswith("3Key"):
            key = type(key)("Key" + key[4:])
        drafts.append(
            NoteDraft(
                down_us=round(int(note.time_ms) * 1000 / tempo_scale),
                scan_code=resolver.resolve_scan_code(key, "physical"),
            )
        )
    deduped: list[NoteDraft] = []
    seen_slots: set[tuple[int, int]] = set()
    for draft in sorted(drafts, key=lambda item: item.down_us):
        slot = (draft.down_us, draft.scan_code)
        if slot in seen_slots:
            continue
        seen_slots.add(slot)
        deduped.append(draft)
    return tuple(deduped)


def next_same_intervals(drafts: tuple[NoteDraft, ...]) -> tuple[int, ...]:
    intervals: list[int] = []
    last_seen_by_key: dict[int, int] = {}
    for draft in reversed(drafts):
        next_same_us = last_seen_by_key.get(draft.scan_code)
        if next_same_us is not None:
            intervals.append(next_same_us - draft.down_us)
        last_seen_by_key[draft.scan_code] = draft.down_us
    return tuple(intervals)


def peak_notes_per_second(drafts: tuple[NoteDraft, ...]) -> int:
    if not drafts:
        return 0
    down_times = [draft.down_us for draft in drafts]
    left = 0
    peak = 0
    for right, down_us in enumerate(down_times):
        while down_us - down_times[left] > 1_000_000:
            left += 1
        peak = max(peak, right - left + 1)
    return peak


def scheduled_repeat_gaps(actions: tuple[KeyAction, ...]) -> tuple[int, ...]:
    last_up_by_key: dict[int, int] = {}
    gaps: list[int] = []
    for action in actions:
        for scan_code in action.scan_codes:
            if action.kind == "up":
                last_up_by_key[int(scan_code)] = int(action.at_us)
            elif int(scan_code) in last_up_by_key:
                gaps.append(int(action.at_us) - last_up_by_key[int(scan_code)])
    return tuple(gaps)


def audit_song(
    song_path: Path,
    policy: FrameTimingPolicy,
    tempo_scale: float,
    candidate_repeat_gap_us: int,
) -> SongRepeatStats:
    song = parse_song_file(song_path, SKY_15_KEY_PROFILE)
    metadata = build_key_actions(song, policy=policy, tempo_scale=tempo_scale)
    drafts = resolve_drafts(song_path, tempo_scale)
    intervals = next_same_intervals(drafts)
    cycle_us = int(policy.min_hold_us) + candidate_repeat_gap_us
    binding_limit_us = int(policy.hold_us) + candidate_repeat_gap_us

    return SongRepeatStats(
        path=song_path,
        note_count=len(song.notes),
        peak_notes_per_second=peak_notes_per_second(drafts),
        repeat_candidates=len(intervals),
        binding_count=sum(1 for interval in intervals if interval < binding_limit_us),
        positive_binding_count=sum(1 for interval in intervals if 0 < interval < binding_limit_us),
        compressible_binding_count=sum(
            1 for interval in intervals if cycle_us <= interval < binding_limit_us
        ),
        under_cycle_count=sum(1 for interval in intervals if interval < cycle_us),
        positive_under_cycle_count=sum(1 for interval in intervals if 0 < interval < cycle_us),
        zero_interval_count=sum(1 for interval in intervals if interval == 0),
        compressed_holds=metadata.compressed_holds,
        impossible_repeats=metadata.impossible_same_key_repeats,
        duplicate_note_count=metadata.duplicate_note_count,
        min_same_key_up_gap_us=metadata.min_same_key_up_gap_us,
        min_same_key_interval_us=min(intervals, default=None),
        min_positive_same_key_interval_us=min(
            (interval for interval in intervals if interval > 0),
            default=None,
        ),
        actual_repeat_gaps_us=scheduled_repeat_gaps(metadata.actions),
    )


def audit_corpus(
    songs_dir: Path,
    policy: FrameTimingPolicy,
    tempo_scale: float,
    candidate_repeat_gap_us: int,
    song_path: Path | None = None,
) -> CorpusRepeatStats:
    songs: list[SongRepeatStats] = []
    failed_files: list[tuple[Path, str]] = []
    paths = (song_path,) if song_path is not None else collect_song_paths(songs_dir)
    for path in paths:
        try:
            songs.append(audit_song(path, policy, tempo_scale, candidate_repeat_gap_us))
        except Exception as exc:  # noqa: BLE001 - report bad corpus files without aborting the audit.
            failed_files.append((path, str(exc)))
    return CorpusRepeatStats(songs=tuple(songs), failed_files=tuple(failed_files))


def print_totals(label: str, songs: tuple[SongRepeatStats, ...]) -> None:
    repeat_candidates = sum(song.repeat_candidates for song in songs)
    binding_count = sum(song.binding_count for song in songs)
    positive_binding_count = sum(song.positive_binding_count for song in songs)
    compressible_binding_count = sum(song.compressible_binding_count for song in songs)
    under_cycle_count = sum(song.under_cycle_count for song in songs)
    positive_under_cycle_count = sum(song.positive_under_cycle_count for song in songs)
    zero_interval_count = sum(song.zero_interval_count for song in songs)
    compressed_holds = sum(song.compressed_holds for song in songs)
    impossible_repeats = sum(song.impossible_repeats for song in songs)
    duplicate_note_count = sum(song.duplicate_note_count for song in songs)
    min_positive_same_key_interval_us = min(
        (
            song.min_positive_same_key_interval_us
            for song in songs
            if song.min_positive_same_key_interval_us is not None
        ),
        default=None,
    )
    min_same_key_up_gap_us = min(
        (
            song.min_same_key_up_gap_us
            for song in songs
            if song.min_same_key_up_gap_us is not None
        ),
        default=None,
    )

    print(label)
    print(f"  repeat candidates: {repeat_candidates}")
    print(f"  repeat-gap binding intervals: {binding_count} ({fmt_pct(binding_count, repeat_candidates)})")
    print(
        "  positive repeat-gap binding intervals: "
        f"{positive_binding_count} ({fmt_pct(positive_binding_count, repeat_candidates)})"
    )
    print(
        "  schedule-changing compression-band intervals: "
        f"{compressible_binding_count} ({fmt_pct(compressible_binding_count, repeat_candidates)})"
    )
    print(f"  under min_hold+repeat_gap cycle: {under_cycle_count} ({fmt_pct(under_cycle_count, repeat_candidates)})")
    print(
        "  positive intervals under cycle: "
        f"{positive_under_cycle_count} ({fmt_pct(positive_under_cycle_count, repeat_candidates)})"
    )
    print(f"  zero-interval same-key duplicates/chords: {zero_interval_count} ({fmt_pct(zero_interval_count, repeat_candidates)})")
    print(f"  exact same-key timestamp duplicates removed: {duplicate_note_count}")
    print(f"  runtime compressed holds: {compressed_holds} ({fmt_pct(compressed_holds, repeat_candidates)})")
    print(f"  impossible repeats: {impossible_repeats}")
    print(f"  minimum positive same-key interval: {fmt_us(min_positive_same_key_interval_us)}")
    print(f"  minimum actual same-key up gap: {fmt_us(min_same_key_up_gap_us)}")


def print_summary(
    corpus: CorpusRepeatStats,
    policy: FrameTimingPolicy,
    *,
    tempo_scale: float,
    candidate_repeat_gap_us: int,
    top: int,
) -> None:
    print(
        f"Policy: {policy.profile_name or 'custom'} @ {policy.fps or 'raw'}fps | "
        f"hold={fmt_us(int(policy.hold_us))} min_hold={fmt_us(int(policy.min_hold_us))} "
        f"candidate_repeat_gap={fmt_us(candidate_repeat_gap_us)} tempo={tempo_scale:.3f}x"
    )
    compression_low_us = int(policy.min_hold_us) + candidate_repeat_gap_us
    compression_high_us = int(policy.hold_us) + candidate_repeat_gap_us
    if compression_low_us >= compression_high_us:
        print("Compression band: empty (hold == min_hold); repeat gap cannot alter degraded playback schedule")
    else:
        print(
            "Compression band: "
            f"[{fmt_us(compression_low_us)}, {fmt_us(compression_high_us)}) same-key interval"
        )
    note_count = sum(song.note_count for song in corpus.songs)
    print(f"Parsed songs: {len(corpus.songs)} | failed files: {len(corpus.failed_files)} | notes: {note_count}")
    if len(corpus.songs) == 1:
        short_actual_gaps = sorted(
            {gap for gap in corpus.songs[0].actual_repeat_gaps_us if gap <= 100_000}
        )
        print(
            "Actual scheduled repeat gaps <=100ms: "
            f"{[fmt_us(gap) for gap in short_actual_gaps]}"
        )
    print()
    print_totals("All parsed songs", corpus.songs)
    real_songs = tuple(song for song in corpus.songs if not song.path.stem.startswith("TEST_"))
    print()
    print_totals("Real songs only (excluding TEST_*)", real_songs)

    sensitive = sorted(
        (song for song in corpus.songs if song.binding_count > 0),
        key=lambda song: (
            song.positive_binding_count,
            song.positive_under_cycle_count,
            song.binding_count,
        ),
        reverse=True,
    )[:top]
    print()
    print(f"Top {top} same-key-cycle-sensitive songs")
    if not sensitive:
        print("none")
    for song in sensitive:
        short_actual_gaps = sorted({gap for gap in song.actual_repeat_gaps_us if gap <= 100_000})
        print(
            f"{song.path.name}: binds={song.binding_count}/{song.repeat_candidates}, "
            f"positive_binds={song.positive_binding_count}, "
            f"compressible={song.compressible_binding_count}, "
            f"under_cycle={song.under_cycle_count}, "
            f"positive_under_cycle={song.positive_under_cycle_count}, "
            f"zero={song.zero_interval_count}, impossible={song.impossible_repeats}, "
            f"deduped={song.duplicate_note_count}, "
            f"min_same={fmt_us(song.min_same_key_interval_us)}, "
            f"min_positive={fmt_us(song.min_positive_same_key_interval_us)}, "
            f"min_up_gap={fmt_us(song.min_same_key_up_gap_us)}, "
            f"actual_short_gaps={[fmt_us(gap) for gap in short_actual_gaps]}"
        )

    dense = sorted(corpus.songs, key=lambda song: (song.peak_notes_per_second, song.note_count), reverse=True)[:top]
    print()
    print(f"Top {top} dense/fast songs")
    print(
        "song | notes | peak/s | min_same | min_positive | binds | "
        "positive_binds | compressible | under_cycle"
    )
    for song in dense:
        print(
            f"{song.path.name} | {song.note_count} | {song.peak_notes_per_second} | "
            f"{fmt_us(song.min_same_key_interval_us)} | {fmt_us(song.min_positive_same_key_interval_us)} | "
            f"{song.binding_count}/{song.repeat_candidates} | "
            f"{song.positive_binding_count}/{song.repeat_candidates} | "
            f"{song.compressible_binding_count}/{song.repeat_candidates} | "
            f"{song.under_cycle_count}/{song.repeat_candidates}"
        )

    if corpus.failed_files:
        print()
        print("Failed files")
        for path, error in corpus.failed_files[:top]:
            print(f"{path.name}: {error}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Audit same-key repeat timing pressure across songs.")
    parser.add_argument("--songs-dir", type=Path, default=ROOT / "songs")
    parser.add_argument("--song", type=Path, help="Audit one song and print its actual scheduled gaps.")
    parser.add_argument(
        "--profile",
        default="local-precise",
        choices=("local-precise", "balanced", "dense-safe", "audience-safe"),
    )
    parser.add_argument("--fps", type=int, default=144)
    parser.add_argument("--tempo-scale", type=float, default=1.0)
    parser.add_argument(
        "--repeat-gap-ms",
        type=float,
        default=17.0,
        help="Counterfactual same-key gap candidate in ms. This is not a runtime policy field.",
    )
    parser.add_argument("--hold-ms", type=float, help="Override hold_us for a preflight policy.")
    parser.add_argument("--min-hold-ms", type=float, help="Override min_hold_us for a preflight policy.")
    parser.add_argument("--top", type=int, default=12)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.tempo_scale <= 0:
        raise ValueError("--tempo-scale must be > 0")
    overrides_list: list[tuple[str, int]] = []
    if args.hold_ms is not None:
        overrides_list.append(("hold_us", round(args.hold_ms * 1000)))
    if args.min_hold_ms is not None:
        overrides_list.append(("min_hold_us", round(args.min_hold_ms * 1000)))
    candidate_repeat_gap_us = round(args.repeat_gap_ms * 1000)
    policy = PlaybackSessionContext(
        profile_name=args.profile,
        fps=args.fps,
        policy_overrides=tuple(overrides_list),
    ).resolve_effective_policy(AppConfig())
    song_path = args.song.resolve() if args.song is not None else None
    if song_path is not None and not song_path.is_file():
        raise FileNotFoundError(song_path)
    corpus = audit_corpus(
        args.songs_dir,
        policy,
        args.tempo_scale,
        candidate_repeat_gap_us,
        song_path,
    )
    print_summary(
        corpus,
        policy,
        tempo_scale=args.tempo_scale,
        candidate_repeat_gap_us=candidate_repeat_gap_us,
        top=args.top,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

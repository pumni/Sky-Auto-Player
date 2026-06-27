"""Onset / IOI analysis for in-game timing validation.

Usage:
    uv run python tests/analyze_onsets.py labels.txt [telemetry.csv] [--per-block] [--gap-ms N]

`labels.txt` is an Audacity label export (tab-separated; column 0 = onset start, seconds).
Passing the telemetry CSV adds a SENDER AUDIT and the game-only jitter calculation.

IMPORTANT: the [SENT] / game-only figures only count downs the runtime ACTUALLY sent
(`sent_scan_codes` non-empty). Rows the runtime recorded but dropped (dropped_conflict,
dropped_expired) or suppressed (suppressed_stale_up) are excluded — otherwise phantom
"sends" would corrupt the IOI baseline and the audio comparison. The audit prints the
pre-gate so you can tell whether the recorded audio is even valid ground truth: if the
sender did not emit every intended down, an audio onset count cannot be attributed to the
game.

--per-block: multi-block probe songs (e.g. TEST_repeat_clean_*) insert ~1.5 s of silence
between blocks, which inflates a whole-song [SENT] IOI std into the hundreds of ms even when
each block is microsecond-clean. With --per-block the script segments both the sent downs and
the audio onsets at gaps larger than --gap-ms (default 300) and reports IOI per block, which is
the correct granularity for the ~0.05-0.07 ms sender criterion.
"""
import argparse
import csv
import itertools
import statistics


def _segment(times, gap_ms):
    """Split a sorted list of times (seconds) into blocks at gaps larger than gap_ms."""
    if not times:
        return []
    blocks, current = [], [times[0]]
    for prev, cur in itertools.pairwise(times):
        if (cur - prev) * 1000 > gap_ms:
            blocks.append(current)
            current = [cur]
        else:
            current.append(cur)
    blocks.append(current)
    return blocks


def _iois_ms(times):
    return [(times[i + 1] - times[i]) * 1000 for i in range(len(times) - 1)]


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("labels", help="Audacity label export (tab-separated; col0 = onset start, sec)")
    ap.add_argument("csv", nargs="?", help="telemetry CSV from --debug-csv (logs/*.csv)")
    ap.add_argument("--per-block", action="store_true", help="segment by silence and report IOI per block")
    ap.add_argument("--gap-ms", type=float, default=300.0, help="gap (ms) that separates blocks (default 300)")
    args = ap.parse_args()

    starts = sorted(
        float(line.split("\t")[0])
        for line in open(args.labels, encoding="utf-8")
        if line.strip()
    )
    ioi = _iois_ms(starts)
    if ioi:
        print(
            f"[GAME]  onsets={len(starts)}  IOI mean={statistics.mean(ioi):.2f}  "
            f"std={statistics.pstdev(ioi):.2f}  spread={max(ioi) - min(ioi):.2f} ms"
        )
    else:
        print(f"[GAME]  onsets={len(starts)}  (need >=2 onsets for IOI)")

    sent_downs = None
    if args.csv:
        rows = list(csv.DictReader(open(args.csv, encoding="utf-8")))
        down_rows = [r for r in rows if r["kind"] == "down"]

        def _sent(r) -> bool:
            return bool(r.get("sent_scan_codes", "").strip())

        sent_down_rows = [r for r in down_rows if _sent(r)]

        def _count(outcome: str) -> int:
            return sum(1 for r in down_rows if r.get("runtime_outcome") == outcome)

        intended, sent = len(down_rows), len(sent_down_rows)
        print(
            f"[SENDER AUDIT]  intended_down={intended}  sent_down={sent}  "
            f"dropped_conflict={_count('dropped_conflict')}  dropped_expired={_count('dropped_expired')}  "
            f"suppressed_stale_up={sum(1 for r in rows if r.get('runtime_outcome') == 'suppressed_stale_up')}"
        )
        if sent != intended:
            print(
                f"[GATE]  ** sender did NOT emit every intended down ({sent}/{intended}) **\n"
                f"        Audio onset count is NOT valid ground truth: the {intended - sent} missing "
                f"note(s) were lost BEFORE the game.\n"
                f"        A block at the frame floor (e.g. 7 ms @144fps, ~55 us headroom) dropping is a "
                f"tempo/profile signal, not an anchor bug; a block with comfortable headroom dropping IS."
            )
        else:
            print(f"[GATE]  OK - sender emitted all {intended} intended downs; audio is valid ground truth.")

        sent_downs = [int(r["actual_us"]) / 1_000_000 for r in sent_down_rows]  # seconds
        sent_ioi = _iois_ms(sent_downs)
        if sent_ioi:
            print(
                f"[SENT]  IOI std={statistics.pstdev(sent_ioi):.2f} ms (whole-song; "
                f"inflated by inter-block silence — use --per-block)"
            )

        n = min(len(ioi), len(sent_ioi))
        if n > 0 and not args.per_block:
            game_only = [ioi[i] - sent_ioi[i] for i in range(n)]
            mean_go = statistics.mean(game_only)
            print(
                f"[GAME-only jitter] std={statistics.pstdev(game_only):.2f}  "
                f"spread={max(game_only) - min(game_only):.2f} ms"
            )
            print("residuals(ms):", " ".join(f"{x - mean_go:+.1f}" for x in game_only))
            if sent != intended:
                print(
                    "[GAME-only jitter] ** suspect: aligning audio onsets to sent downs by index is "
                    "unreliable when the sender dropped events. **"
                )

    if args.per_block:
        onset_blocks = _segment(starts, args.gap_ms)
        print(f"\n[PER-BLOCK]  gap-ms={args.gap_ms:g}  audio_blocks={len(onset_blocks)}", end="")
        sent_blocks = _segment(sent_downs, args.gap_ms) if sent_downs is not None else None
        if sent_blocks is not None:
            print(f"  sent_blocks={len(sent_blocks)}")
            if len(sent_blocks) != len(onset_blocks):
                print(
                    "  ** block counts differ — cannot align audio to sender per block; "
                    "check detector / gap-ms. Showing audio-only IOI. **"
                )
                sent_blocks = None
        else:
            print()

        header = f"  {'blk':>3} {'onsets':>6} {'GAME mean':>9} {'GAME std':>8}"
        if sent_blocks is not None:
            header += f" {'SENT std':>8} {'game-only std':>13}"
        print(header)
        for i, ob in enumerate(onset_blocks):
            g_ioi = _iois_ms(ob)
            g_mean = statistics.mean(g_ioi) if g_ioi else 0.0
            g_std = statistics.pstdev(g_ioi) if len(g_ioi) > 1 else 0.0
            line = f"  {i + 1:>3} {len(ob):>6} {g_mean:>9.3f} {g_std:>8.4f}"
            if sent_blocks is not None:
                s_ioi = _iois_ms(sent_blocks[i])
                s_std = statistics.pstdev(s_ioi) if len(s_ioi) > 1 else 0.0
                m = min(len(g_ioi), len(s_ioi))
                go = [g_ioi[j] - s_ioi[j] for j in range(m)]
                go_std = statistics.pstdev(go) if len(go) > 1 else 0.0
                line += f" {s_std:>8.4f} {go_std:>13.4f}"
            print(line)


if __name__ == "__main__":
    main()

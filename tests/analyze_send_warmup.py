"""SENDER-WARMUP ANALYSIS — does a cold CPU core inflate SendInput latency?

This is the *sender-side* counterpart to `tests/measure_stutter.py` (which needs recorded audio
and looks at what happens AFTER send).  Here we never leave the telemetry CSV: we test the
hypothesis that sleeping between notes lets the dispatch core downclock/park, so that the
`SendInput` call right after a long idle gap runs slower (higher `send_duration_us`) — a
before-send source of jitter that the spin re-anchor cannot hide.

The engine records two observe-only columns per send:
    idle_gap_us       how long the dispatch thread idled/slept before the final warm-up spin
    pre_send_spin_us  how long it busy-spun (warming the core) right before SendInput

For one or more runs this script prints:
  * send_duration distribution overall,
  * send_duration bucketed by idle_gap (cold rows should be slower if the hypothesis holds),
  * a cold-vs-warm split with the Pearson correlation of idle_gap vs send_duration,
  * and, when several CSVs are given, a side-by-side comparison so you can read off whether
    forcing the core warm (min processor state = 100%) or widening the spin window helps.

USAGE
    # one run
    uv run python tests/analyze_send_warmup.py logs/playback_telemetry_<id>.csv

    # compare the three configs from the experiment plan
    uv run python tests/analyze_send_warmup.py \
        logs/balanced.csv logs/min100.csv logs/spin3000.csv \
        --labels balanced,min100,spin3000

Record each run identically except the one variable under test, e.g.:
    uv run play --song "blue" --fps 144 --timing-profile local-precise --debug-csv
"""
from __future__ import annotations

import argparse
import csv
import math
import statistics
from pathlib import Path

COLD_THRESHOLD_US = 20_000
# idle_gap buckets (microseconds) for the per-bucket send_duration breakdown.
BUCKET_EDGES_US = [0, 1_000, 5_000, 20_000, 50_000, 200_000, math.inf]


def _pct(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    return float(s[int(round(pct * (len(s) - 1)))])


def _stats(values: list[float]) -> dict[str, float]:
    if not values:
        return {"n": 0, "p50": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0, "avg": 0.0}
    return {
        "n": len(values),
        "p50": _pct(values, 0.50),
        "p95": _pct(values, 0.95),
        "p99": _pct(values, 0.99),
        "max": float(max(values)),
        "avg": sum(values) / len(values),
    }


def _pearson(xs: list[float], ys: list[float]) -> float:
    """Pearson correlation of idle_gap vs send_duration; 0.0 if undefined."""
    n = len(xs)
    if n < 2:
        return 0.0
    mx, my = statistics.mean(xs), statistics.mean(ys)
    sx, sy = statistics.pstdev(xs), statistics.pstdev(ys)
    if sx == 0 or sy == 0:
        return 0.0
    cov = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / n
    return cov / (sx * sy)


def load_sent_rows(csv_path: str) -> list[dict[str, int]]:
    """Return sent dispatch rows with the warmup columns parsed to ints (skips event_index 0).

    event_index 0 is skipped: its idle_gap is measured from a zero baseline and is meaningless.
    """
    rows = list(csv.DictReader(open(csv_path, encoding="utf-8")))
    out: list[dict[str, int]] = []
    for r in rows:
        if not r.get("sent_scan_codes", "").strip():
            continue
        if "idle_gap_us" not in r:
            raise SystemExit(
                f"{csv_path}: no 'idle_gap_us' column — re-record with the instrumented build "
                f"(uv run play ... --debug-csv)."
            )
        try:
            event_index = int(r["event_index"])
        except (KeyError, ValueError):
            event_index = -1
        if event_index == 0:
            continue
        out.append(
            {
                "send_duration_us": int(r["send_duration_us"]),
                "idle_gap_us": int(r["idle_gap_us"]),
                "pre_send_spin_us": int(r.get("pre_send_spin_us", 0)),
                "kind": r["kind"],
            }
        )
    return out


def analyze_one(label: str, rows: list[dict[str, int]]) -> dict:
    sends = [r["send_duration_us"] for r in rows]
    gaps = [r["idle_gap_us"] for r in rows]
    cold = [r["send_duration_us"] for r in rows if r["idle_gap_us"] > COLD_THRESHOLD_US]
    warm = [r["send_duration_us"] for r in rows if r["idle_gap_us"] <= COLD_THRESHOLD_US]

    print(f"\n{'='*78}\n{label}  ({len(rows)} sent dispatches)\n{'='*78}")

    overall = _stats([float(v) for v in sends])
    print(
        f"send_duration_us  : p50={overall['p50']:.0f}  p95={overall['p95']:.0f}  "
        f"p99={overall['p99']:.0f}  max={overall['max']:.0f}  avg={overall['avg']:.0f}"
    )

    print("\n  send_duration_us by idle_gap bucket (cold rows = larger gaps):")
    print(f"    {'idle_gap':>16}  {'n':>5}  {'p50':>6}  {'p95':>6}  {'p99':>6}  {'max':>7}")
    for lo, hi in zip(BUCKET_EDGES_US, BUCKET_EDGES_US[1:]):
        bucket = [float(r["send_duration_us"]) for r in rows if lo <= r["idle_gap_us"] < hi]
        if not bucket:
            continue
        hi_label = "inf" if hi == math.inf else f"{hi/1000:.0f}ms"
        rng = f"{lo/1000:.0f}-{hi_label}"
        st = _stats(bucket)
        print(
            f"    {rng:>16}  {st['n']:>5}  {st['p50']:>6.0f}  {st['p95']:>6.0f}  "
            f"{st['p99']:>6.0f}  {st['max']:>7.0f}"
        )

    cold_st, warm_st = _stats([float(v) for v in cold]), _stats([float(v) for v in warm])
    r = _pearson([float(g) for g in gaps], [float(s) for s in sends])
    print(
        f"\n  COLD (idle>{COLD_THRESHOLD_US/1000:.0f}ms): n={cold_st['n']:<5} "
        f"p95={cold_st['p95']:.0f} p99={cold_st['p99']:.0f} max={cold_st['max']:.0f}"
    )
    print(
        f"  WARM (idle<={COLD_THRESHOLD_US/1000:.0f}ms): n={warm_st['n']:<5} "
        f"p95={warm_st['p95']:.0f} p99={warm_st['p99']:.0f} max={warm_st['max']:.0f}"
    )
    print(f"  Pearson r(idle_gap, send_duration) = {r:+.3f}  (~0 = no link, >0 = cold inflates send)")

    return {
        "label": label,
        "overall": overall,
        "cold": cold_st,
        "warm": warm_st,
        "pearson": r,
    }


def print_comparison(results: list[dict]) -> None:
    if len(results) < 2:
        return
    print(f"\n{'='*78}\nCOMPARISON\n{'='*78}")
    print(
        f"{'config':>14}  {'send_p95':>8}  {'send_p99':>8}  {'cold_p99':>8}  "
        f"{'warm_p99':>8}  {'pearson':>7}"
    )
    for res in results:
        print(
            f"{res['label']:>14}  {res['overall']['p95']:>8.0f}  {res['overall']['p99']:>8.0f}  "
            f"{res['cold']['p99']:>8.0f}  {res['warm']['p99']:>8.0f}  {res['pearson']:>+7.3f}"
        )
    print(
        "\nReading: if a config drops cold_p99 (and overall p99) toward warm_p99 while pearson\n"
        "falls toward 0, the cold-core hypothesis holds and that config is the cure. If cold_p99\n"
        "~= warm_p99 and pearson ~= 0 everywhere, send latency is independent of sleep — the\n"
        "stutter is elsewhere (game/driver), so leave the dispatch sleep model alone."
    )


def main() -> int:
    try:
        import sys

        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("csv", nargs="+", help="telemetry CSV(s) from --debug-csv")
    ap.add_argument("--labels", default="", help="comma-separated labels parallel to the CSVs")
    args = ap.parse_args()

    labels = [s.strip() for s in args.labels.split(",")] if args.labels else []
    results = []
    for i, path in enumerate(args.csv):
        label = labels[i] if i < len(labels) and labels[i] else Path(path).stem
        rows = load_sent_rows(path)
        if not rows:
            print(f"{label}: no sent dispatches with warmup columns; skipping.")
            continue
        results.append(analyze_one(label, rows))

    print_comparison(results)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

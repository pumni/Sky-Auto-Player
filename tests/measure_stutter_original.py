"""§1 STUTTER MEASUREMENT — correlate injected events with the notes actually heard.

This is the *direct* measurement called for in `docs/timing-principles.md` §1 (this
investigation) and Appendix A.1: ground truth is the **recorded game audio**, not the
scheduler log. The player's telemetry only proves the *send* side; it is blind to anything
that happens after `SendInput()` returns (OS input delivery, the game's frame sampler, the
game's audio engine). This script closes that gap by comparing, on the same run:

    [SENT]  when the runtime actually injected each down  (telemetry CSV, actual_us)
    [HEARD] when a note actually sounded                  (onsets detected in the WAV)

and produces the three decisive verdicts that split "before-send" from "after-send":

  1. MISSING NOTES  — downs the runtime emitted but that produced NO audio onset.
     => the note was lost AFTER the player (OS delivery dropped it, or the game saw a
        sub-frame hold and never registered it). This is the "game không nhận" case.
  2. STUTTER GAPS   — pairs where the audio inter-onset interval is much larger than the
     scheduled interval, with the song timestamp printed so you can confirm "yes, that is
     where I heard the hitch". A gap here with a CLEAN telemetry IOI = the stall is
     after-send. A gap that ALSO shows in telemetry lateness = before-send (scheduler/OS).
  3. GAME-ONLY JITTER — std/spread of (heard - sent) after removing the constant recording
     offset. Small (~<3 ms) = clean. Bimodal / ±~20 ms = the game-side bucket scatter of
     Appendix A.10 (perceived as "uneven", not a true stall), not player-tunable.

--------------------------------------------------------------------------------------------
HOW TO RECORD A RUN (do this first)
--------------------------------------------------------------------------------------------
1. Lock the game FPS externally (RTSS / vsync) so frame timing is constant.
2. Pick a song you RELIABLY hear stutter on. A percussive instrument in-game gives sharp
   onsets that are easy to detect.
3. Start recording the game audio in Audacity (or any recorder) -> export later as
   **16-bit PCM WAV** (Audacity: File > Export > WAV, "WAV signed 16-bit PCM").
4. In another terminal, play WITH telemetry:

       uv run play --song "blue" --fps 144 --timing-profile local-precise --debug-csv

   (Use the SAME song/fps/profile you actually hear stutter with. --debug-csv writes
   logs/playback_telemetry_<id>.csv — note the newest one.)
5. Stop the recording AFTER the song ends. Right after, note out loud / in a file roughly
   WHERE you heard the hitches (e.g. "around 0:08 and 0:21") — that subjective label is
   what you cross-check against this script's STUTTER GAPS section.

--------------------------------------------------------------------------------------------
RUN THE ANALYSIS
--------------------------------------------------------------------------------------------
    uv run python tests/measure_stutter.py recording.wav logs/playback_telemetry_<id>.csv --fps 144

You can also pass an Audacity label export (.txt) instead of a .wav if you prefer to mark
onsets by hand:  uv run python tests/measure_stutter.py labels.txt logs/<id>.csv --fps 144

Tuning the detector if the onset count is wrong:
  --sensitivity 1.5   higher = fewer onsets (raise if it finds spurious onsets)
  --min-gap-ms 40     refractory period; raise if one note is detected twice
  --hop-ms 5          envelope resolution
"""
from __future__ import annotations

import argparse
import contextlib
import csv
import math
import statistics
import sys
import wave
from array import array


# --------------------------------------------------------------------------------------------
# WAV reading + onset detection (pure stdlib; audioop was removed in 3.13+)
# --------------------------------------------------------------------------------------------
def read_wav_mono(path: str) -> tuple[list[float], int]:
    """Return (mono_samples in [-1,1], sample_rate). 16-bit PCM is the supported default."""
    with wave.open(path, "rb") as w:
        n_channels = w.getnchannels()
        sampwidth = w.getsampwidth()
        sr = w.getframerate()
        n_frames = w.getnframes()
        raw = w.readframes(n_frames)

    if sampwidth == 2:
        data = array("h")
        data.frombytes(raw)
        scale = 1.0 / 32768.0
    elif sampwidth == 4:
        data = array("i")
        data.frombytes(raw)
        scale = 1.0 / 2147483648.0
    elif sampwidth == 1:
        # 8-bit WAV is unsigned, centred at 128
        data = array("b")
        data.frombytes(bytes((b ^ 0x80) for b in raw))  # unsigned 8-bit -> signed
        scale = 1.0 / 128.0
    else:
        raise SystemExit(
            f"Unsupported WAV sample width {sampwidth*8}-bit. "
            f"Re-export as 16-bit PCM WAV (Audacity: 'WAV signed 16-bit PCM')."
        )

    if n_channels == 1:
        mono = [s * scale for s in data]
    else:
        # average interleaved channels
        mono = [0.0] * (len(data) // n_channels)
        for i in range(len(mono)):
            acc = 0
            base = i * n_channels
            for c in range(n_channels):
                acc += data[base + c]
            mono[i] = (acc / n_channels) * scale
    return mono, sr


def detect_onsets(
    samples: list[float], sr: int, *, hop_ms: float, min_gap_ms: float, sensitivity: float
) -> list[float]:
    """Energy-novelty onset detector. Returns onset times in seconds.

    Frames the signal into non-overlapping hops, takes per-frame RMS, builds a half-wave
    rectified novelty (rise in RMS), normalises it, then peak-picks above an adaptive local
    threshold with a refractory period. Tuned for percussive onsets.
    """
    hop = max(1, int(sr * hop_ms / 1000.0))
    n = len(samples)
    n_frames = n // hop
    if n_frames < 2:
        return []

    # per-frame RMS
    rms = [0.0] * n_frames
    for i in range(n_frames):
        base = i * hop
        acc = 0.0
        for k in range(hop):
            v = samples[base + k]
            acc += v * v
        rms[i] = math.sqrt(acc / hop)

    # half-wave rectified novelty (rise in RMS)
    nov = [0.0] * n_frames
    for i in range(1, n_frames):
        d = rms[i] - rms[i - 1]
        nov[i] = d if d > 0 else 0.0

    peak = max(nov) if nov else 0.0
    if peak <= 0:
        return []
    nov = [x / peak for x in nov]  # normalise to [0,1]

    # adaptive threshold: local mean over a window + sensitivity * local spread
    win = max(3, int(0.150 * sr / hop))  # ~150 ms window
    onsets: list[float] = []
    last_onset_frame = -10**9
    refractory = max(1, int(min_gap_ms / hop_ms))
    base_floor = 0.02  # ignore noise floor
    for i in range(1, n_frames - 1):
        lo = max(0, i - win)
        hi = min(n_frames, i + win)
        local = nov[lo:hi]
        mean = sum(local) / len(local)
        thr = mean * sensitivity + base_floor
        if (
            nov[i] > thr
            and nov[i] >= nov[i - 1]
            and nov[i] >= nov[i + 1]
            and i - last_onset_frame >= refractory
        ):
            onsets.append(i * hop / sr)
            last_onset_frame = i
    return onsets


def read_label_onsets(path: str) -> list[float]:
    """Audacity label export: tab-separated, column 0 = onset start in seconds."""
    return sorted(
        float(line.split("\t")[0])
        for line in open(path, encoding="utf-8")
        if line.strip()
    )


# --------------------------------------------------------------------------------------------
# Telemetry (sent downs) + sender gate  (mirrors tests/analyze_onsets.py)
# --------------------------------------------------------------------------------------------
def load_sent_downs(csv_path: str) -> tuple[list[float], int, dict[str, int]]:
    """Return (sent_down_times_sec, intended_down_count, outcome_counts)."""
    rows = list(csv.DictReader(open(csv_path, encoding="utf-8")))
    down_rows = [r for r in rows if r["kind"] == "down"]

    def sent(r) -> bool:
        return bool(r.get("sent_scan_codes", "").strip())

    sent_rows = [r for r in down_rows if sent(r)]
    counts = {
        "intended": len(down_rows),
        "sent": len(sent_rows),
        "dropped_conflict": sum(1 for r in down_rows if r.get("runtime_outcome") == "dropped_conflict"),
        "dropped_expired": sum(1 for r in down_rows if r.get("runtime_outcome") == "dropped_expired"),
        "suppressed_stale_up": sum(1 for r in rows if r.get("runtime_outcome") == "suppressed_stale_up"),
    }
    times = [int(r["actual_us"]) / 1_000_000 for r in sent_rows]
    return sorted(times), len(down_rows), counts


# --------------------------------------------------------------------------------------------
# Alignment: find the constant recording offset, then nearest-match sent <-> heard
# --------------------------------------------------------------------------------------------
def best_offset(sent: list[float], heard: list[float], tol: float) -> float:
    """Search the constant offset that maximises matched pairs (tie-break: min abs error).

    offset is defined so that  heard ~= sent + offset. Candidates come from pairing the
    first few sends against the first few onsets, which covers the unknown record start.
    """
    if not sent or not heard:
        return 0.0
    k = min(15, len(sent), len(heard))
    candidates = sorted({round(heard[j] - sent[i], 4) for i in range(k) for j in range(k)})
    h = sorted(heard)
    best = (-1, float("inf"), 0.0)  # (matched, err, offset)
    for off in candidates:
        matched = 0
        err = 0.0
        for s in sent:
            target = s + off
            best_d = min((abs(x - target) for x in h), default=float("inf"))
            if best_d <= tol:
                matched += 1
                err += best_d
        if (matched, -err) > (best[0], -best[1]):
            best = (matched, err, off)
    return best[2]


def match(sent: list[float], heard: list[float], offset: float, tol: float):
    """Greedy nearest match. Returns (pairs, missing_sent_idx, extra_heard_idx).

    pairs = list of (sent_idx, heard_idx, sent_time, heard_time_aligned)
    """
    used = [False] * len(heard)
    pairs = []
    missing = []
    for si, s in enumerate(sent):
        target = s + offset
        best_j, best_d = -1, tol
        for hj, ht in enumerate(heard):
            if used[hj]:
                continue
            d = abs(ht - target)
            if d <= best_d:
                best_d, best_j = d, hj
        if best_j >= 0:
            used[best_j] = True
            pairs.append((si, best_j, s, heard[best_j] - offset))
        else:
            missing.append(si)
    extra = [hj for hj, u in enumerate(used) if not u]
    return pairs, missing, extra


def fmt_t(sec: float) -> str:
    m = int(sec // 60)
    s = sec - m * 60
    return f"{m}:{s:05.2f}"


def main() -> int:
    with contextlib.suppress(Exception):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("audio", help="recorded game audio (.wav) OR Audacity label export (.txt)")
    ap.add_argument("csv", help="telemetry CSV from --debug-csv (logs/*.csv)")
    ap.add_argument("--fps", type=int, default=0, help="game FPS, to print the frame period for context")
    ap.add_argument("--tol-ms", type=float, default=120.0,
                    help="match tolerance heard<->sent; must exceed the largest real stutter so a "
                         "late onset is matched-but-late, not miscounted as missing (default 120). "
                         "Lower it for very dense songs where notes are <250 ms apart.")
    ap.add_argument("--gap-ms", type=float, default=30.0,
                    help="report a STUTTER GAP when audio IOI exceeds scheduled IOI by this much (default 30)")
    ap.add_argument("--top", type=int, default=15, help="how many largest gaps to list")
    ap.add_argument("--sensitivity", type=float, default=1.5, help="onset detector: higher = fewer onsets")
    ap.add_argument("--min-gap-ms", type=float, default=40.0, help="onset detector refractory period")
    ap.add_argument("--hop-ms", type=float, default=5.0, help="onset detector envelope resolution")
    args = ap.parse_args()

    # ---- heard onsets ----
    if args.audio.lower().endswith(".wav"):
        print(f"Reading WAV {args.audio} ...")
        samples, sr = read_wav_mono(args.audio)
        print(f"  {len(samples)} samples @ {sr} Hz ({len(samples)/sr:.1f}s). Detecting onsets...")
        heard = detect_onsets(
            samples, sr, hop_ms=args.hop_ms, min_gap_ms=args.min_gap_ms, sensitivity=args.sensitivity
        )
    else:
        heard = read_label_onsets(args.audio)
    print(f"[HEARD] {len(heard)} audio onsets detected")

    # ---- sent downs + gate ----
    sent, _intended, counts = load_sent_downs(args.csv)
    print(
        f"[SENDER AUDIT] intended_down={counts['intended']} sent_down={counts['sent']} "
        f"dropped_conflict={counts['dropped_conflict']} dropped_expired={counts['dropped_expired']} "
        f"suppressed_stale_up={counts['suppressed_stale_up']}"
    )
    if counts["sent"] != counts["intended"]:
        print(
            f"[GATE] ** sender did NOT emit every intended down ({counts['sent']}/{counts['intended']}). "
            f"The {counts['intended']-counts['sent']} missing note(s) were lost BEFORE the game — "
            f"audio is not clean ground truth for an after-send verdict. Investigate the scheduler/"
            f"runtime drop first. **"
        )
    else:
        print(f"[GATE] OK — sender emitted all {counts['intended']} intended downs; audio is valid ground truth.")

    if args.fps:
        print(f"[CONTEXT] fps={args.fps}  one frame = {1e6/args.fps/1000:.2f} ms")

    if not sent or not heard:
        print("Not enough data to align (need both sent downs and audio onsets).")
        return 1

    # ---- VALIDITY GATE: onset count must be plausible vs sent count ----
    # If the detector finds far more (or fewer) onsets than notes sent, the recording is not
    # clean ground truth: a spurious-onset bed lets the matcher attach any note to a nearby blip,
    # so "missing" and "gaps" become artifacts. This is Appendix A.7 / hypothesis F (measurement
    # error) and MUST be cleared before any verdict is trusted.
    ratio = len(heard) / max(1, len(sent))
    if not (0.7 <= ratio <= 1.5):
        print(
            f"\n[VALIDITY GATE] ** INVALID RECORDING — {len(heard)} onsets for {len(sent)} sent notes "
            f"(ratio {ratio:.2f}, want 0.7–1.5). The verdicts below are NOT trustworthy. **\n"
            f"   Too many onsets => sustained/melodic instrument + reverb, captured in-game background\n"
            f"   music/ambience, or a dense/polyphonic song smearing notes together. Too few => the\n"
            f"   instrument is too quiet or notes overlap.\n"
            f"   FIX (per Appendix A.1): re-record with a PERCUSSIVE instrument (fast decay), MUTE the\n"
            f"   in-game music/ambience, and use a CONTROLLED probe with well-separated notes, e.g.\n"
            f"   `uv run play --song TEST_metro_alt_200 --fps 144 --timing-profile local-precise --debug-csv`\n"
            f"   (or TEST_visibility). Validate the mechanism on a clean probe first, THEN dense songs.\n"
            f"   You can also try tuning the detector (--sensitivity / --min-gap-ms) but a continuous\n"
            f"   energy bed cannot be fixed by thresholds — fix the recording.\n"
        )

    # ---- align ----
    tol = args.tol_ms / 1000.0
    offset = best_offset(sent, heard, tol)
    pairs, missing, extra = match(sent, heard, offset, tol)
    # Refine: re-centre the offset on the median residual of matched pairs (removes the
    # coarse-search bias and any constant clock skew), then re-match once.
    if pairs:
        med = statistics.median((h - s) for _, _, s, h in pairs)
        offset += med
        pairs, missing, extra = match(sent, heard, offset, tol)
    print(
        f"\n[ALIGN] recording offset = {offset*1000:.1f} ms; "
        f"matched {len(pairs)}/{len(sent)} sent downs to onsets "
        f"(tol ±{args.tol_ms:.0f} ms); unmatched onsets={len(extra)}"
    )

    # ---- 1. MISSING NOTES (sent but no audio) ----
    print(f"\n=== 1. MISSING NOTES (sent by player, NO audio onset) : {len(missing)} ===")
    if missing:
        print("   These notes left the player but never sounded -> lost AFTER send (OS delivery or")
        print("   game saw sub-frame hold). NOTE: a few near same-key fast repeats can be the game's")
        print("   own re-trigger wall (Appendix A.4), not a bug. Listed as song-relative time:")
        t0 = sent[0]
        for si in missing[: args.top]:
            print(f"     sent #{si:<4d} at song t={fmt_t(sent[si]-t0)}  (telemetry actual_us={int(sent[si]*1e6)})")
        if len(missing) > args.top:
            print(f"     ... and {len(missing)-args.top} more")
    else:
        print("   none — every sent down produced an audio onset (no notes lost after send).")

    # ---- 2. STUTTER GAPS (audio IOI >> scheduled IOI) ----
    print("\n=== 2. STUTTER GAPS (audio interval much larger than scheduled) ===")
    # Build consecutive matched pairs to compare IOIs.
    gap_rows = []
    for a in range(len(pairs) - 1):
        si0, _, s0, h0 = pairs[a]
        si1, _, s1, h1 = pairs[a + 1]
        if si1 != si0 + 1:
            continue  # skip across a missing note (IOI undefined)
        sched_ioi = s1 - s0
        heard_ioi = h1 - h0
        excess = heard_ioi - sched_ioi
        gap_rows.append((excess, s0 - sent[0], sched_ioi, heard_ioi))
    big = sorted((g for g in gap_rows if g[0] * 1000 >= args.gap_ms), key=lambda g: -g[0])
    print(f"   {len(big)} gaps exceed +{args.gap_ms:.0f} ms over schedule "
          f"(out of {len(gap_rows)} consecutive intervals)")
    if big:
        print(f"   {'song_t':>8}  {'sched_ms':>8}  {'heard_ms':>8}  {'excess_ms':>9}")
        for excess, songt, sched, heard_ioi in big[: args.top]:
            print(f"   {fmt_t(songt):>8}  {sched*1000:>8.1f}  {heard_ioi*1000:>8.1f}  {excess*1000:>+9.1f}")
        print("   -> Cross-check these timestamps against where you HEARD the hitch.")
        print("      If telemetry lateness at these rows is small (see logs CSV), the stall is")
        print("      AFTER send (OS delivery / game). If telemetry is also late there, it is before send.")
    else:
        print("   none — no coarse stalls in the audio. The 'nấc' is then either sub-perceptual")
        print("   timing scatter (see jitter below) or not present in this run.")

    # ---- 3. GAME-ONLY JITTER ----
    print("\n=== 3. GAME-ONLY JITTER (heard - sent, offset removed) ===")
    resid = [(h - s) for _, _, s, h in pairs]  # h is already offset-removed in match()
    if len(resid) > 2:
        mean = statistics.mean(resid)
        centred = [(r - mean) * 1000 for r in resid]
        std = statistics.pstdev(centred)
        spread = max(centred) - min(centred)
        print(f"   residual std={std:.2f} ms  spread={spread:.2f} ms  (n={len(resid)})")
        if std < 3.0:
            print("   -> clean: heard timing tracks sent timing. No after-send timing problem.")
        elif spread > 30.0:
            print("   -> large/bimodal scatter (~tens of ms): consistent with the game-side bucket")
            print("      jumps of Appendix A.10. Perceived as 'uneven', NOT a player-side stall;")
            print("      not fixable from the player (lead/frame-align were removed for this reason).")
        else:
            print("   -> moderate scatter; inspect the STUTTER GAPS and telemetry lateness to localise.")
    else:
        print("   not enough matched pairs for jitter stats.")

    print("\nDONE. Decision rule (docs/timing-principles.md §1):")
    print("  missing>0 with GATE OK            -> notes lost AFTER send (OS delivery / sub-frame at game)")
    print("  gaps with CLEAN telemetry IOI     -> stall is after send (OS/game), player is innocent")
    print("  gaps that ALSO show in telemetry  -> stall is before send (scheduler/thread/timer/C-state)")
    print("  only jitter, no gaps/missing      -> game-side scatter (A.10), not a true stutter")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: audit cơ chế timing-guard cũ nhắc các knob đã xoá.

# Timing Guard Binding Audit

Date: 2026-06-04

Decision: remove the general release-gap knob completely.

The previous release-gap audit showed that the general post-release guard was a
real code path but practically non-binding in the song corpus. It also made the
profile model harder to reason about because dense Sky songs are dense through
chords, 80-150ms note motion, and same-key repeats, not through cross-key downs a
few milliseconds after a release.

After removing the general release gap and completing the follow-up scheduler-core pass, the model
now exposes:

- `min_hold_us`: key-down visibility floor.

There is no production general release-gap or repeat-release-gap field, CLI override, profile value,
or scheduler delay. Same-key repeat feasibility is now governed by `min_hold_us`: if the authored
same-key interval is below `min_hold_us`, strict mode rejects and degraded mode reports the overlap.

## Repeat-Gap Audit Tool

Use this tool as a counterfactual audit to ask whether a removed candidate same-key gap would have
bound in a corpus:

```powershell
uv run python scripts\audit_repeat_gap.py --profile local-precise --fps 144 --top 12
```

The tool parses `songs/*.json` and `songs/*.skysheet`, uses the existing parser
and scheduler policy materialisation, and reports:

- repeat-gap binding: next same-key interval is below `hold_us + candidate_repeat_gap_us`, so a
  hypothetical normal hold would not leave the candidate gap.
- schedule-changing compression band:
  `min_hold_us + candidate_repeat_gap_us <= interval < hold_us + candidate_repeat_gap_us`.
- impossible same-key cycle pressure: next same-key interval is below
  `min_hold_us + candidate_repeat_gap_us`.
- positive same-key cycle pressure separately from zero-interval duplicate/chord notes.

It also accepts `--tempo-scale` and `--repeat-gap-ms` so production binding can be compared across
supported tempo and candidate floors. For a synthetic preflight, pass `--song`, `--hold-ms`, and
`--min-hold-ms`; the tool prints the effective compression band and short actual scheduled gaps.

## Corpus Finding

Current corpus parsed during the audit after scheduler note-intent normalisation: 110 song files,
76,317 notes, 0 parse failures, including the new synthetic O10/O6 probes.

`local_precise @ 144 FPS` effective policy:

- `hold_us = min_hold_us = 7292`
- historical candidate `repeat_gap_us = 17000` for counterfactual audit only

Real songs only, excluding `TEST_*`, after de-duplicating same-key notes at the same timestamp:

- repeat candidates: 71,964
- positive same-key intervals under cycle: 0 / 71,964, 0.000%
- zero-interval same-key duplicates/chords: 0 / 71,964, 0.000%
- minimum positive same-key interval: 75ms
- schedule-changing compression-band intervals: 0 through tempo 3.0x

The earlier 1843 under-cycle real-song cases were duplicate same-key notes at the exact same
timestamp. They are now normalised before repeat analysis, because they are data/chord duplicates, not
same-key re-trigger attempts.

Current frame-aware profiles materialise `hold_us == min_hold_us`, which makes the compression band
empty by construction. Under the old runtime design, the repeat-gap field therefore could not change
playback schedule in default degraded mode. In the current runtime, that field has been removed; only
the audit script keeps a counterfactual `--repeat-gap-ms` argument.

This means the current real-song corpus does not settle the game's physical same-key gap mechanism,
but it does show that the former `repeat_release_gap_floor_us` is not a reachable production playback
lever under the current frame-aware policy shape.

Synthetic `TEST_repeat_gap.json` does bind positive same-key intervals when the
test is shaped correctly, so targeted same-key material remains the right tool
for O10.4.

## O10.4 Correction

The previous O10.4 instructions used `TEST_repeat_gap` with:

```powershell
--hold-ms 10 --min-hold-ms 10
```

But `TEST_repeat_gap` was authored with `hold_ms = 24`, and each interval is:

```text
interval = 24ms + authored_gap
```

So with a 10ms hold, the actual up-time is:

```text
actual_gap = interval - actual_hold
           = authored_gap + 14ms
```

That means even a former runtime floor of 0ms still left a smallest actual up-time around 19ms in the
old protocol. Passing that test did not prove a 0ms repeat floor was safe.

To measure the floor, O10.4 must either:

- run `TEST_repeat_gap` with `--hold-ms 24` and a visibility-safe `--min-hold-ms`,
  so actual gaps match the authored 50/40/30/24/20/17/14/11/8/5ms blocks; or
- generate a new test song where `interval = measured_hold + target_gap`.

The current generator now provides `TEST_repeat_gap_fine_a/b/c`, authored with hold 24ms and true
0/1/2/3/5/8/11/14/17/20ms gaps in three block orders. Use these for O10.4A. The first note in each
block is a control; a 20-note block contains 19 eligible re-trigger transitions.

Only WAV/game-audio onset counts from the corrected mechanism test **plus** positive production
binding from the corpus audit would justify reintroducing any same-key gap architecture.

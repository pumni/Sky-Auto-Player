# Issue #661 M3 Canary reporting erratum

This append-only erratum corrects interpretation of the original M3 Canary
report. The original run, report, binary, manifest, and checksum inventory are
preserved byte-for-byte. This file does not revise the native evidence or its
hashes.

## Original run identity

- Verdict: one candidate-only `DispatchCleanupM3` Canary session, **PASS**.
- Runner commit: `a1d86ecbd226b14d4a4190839dd8922b512104eb`.
- Candidate source commit/tree: `74c2b2618af50881f15eacd27ab722d316954743` /
  `902c772442706b326e020db6d36daedd60632b02`.
- Workload/scenario: `canonical-single`, Candidate role, Quiet load, 60 FPS,
  500 µs safety margin, no timed performance sample.
- Run ID: `dispatch-core-canary-20261115T171843-18cc80bb`.
- Native case ID: `native-case-20261115T171846-a3a99398`.
- Native process exit code: 0; qualification runner exit code: 0.
- Original candidate binary SHA-256:
  `74986cfd12e9439a3456b17b356cd7cebc7670692ac879e5cdcf81a37c7f0bb0`.
- Original candidate manifest SHA-256:
  `12c45a343ebbb126950a2ce391e3a41508b1bdbea3d1350c10835845cded25f4`.
- Original offline candidate checksum inventory SHA-256:
  `dfdf428b686cb53e5ec786a0edb167d4610a9bd614b8585dceedb6ab8742114e`.
- Original native report SHA-256:
  `3b944907c2eb109f0a3d27acfb4e4f30958f9c04ab845284cb4a192697aff984`.
- Original qualification `manifest.json` SHA-256:
  `307940d5ee3e6b31e8d3185d2be63984145de4b21152aae8a89d25f5b53e9214`.
- Original `summary.json` SHA-256:
  `84668961eb451dd37e430e79110bda9579eefa8ff5bdbc89c34c717b136ac5be`.
- Original raw evidence bundle ZIP SHA-256:
  `01ba89b99230219120d49ed058516a1b060cbe93c66b6b57f586cbfbed292569`.

## Correct H/G interpretation

The pinned native acceptance source defines `ACCEPTANCE_HOLD_FRAMES = 1`.
At 60 FPS, `frame_us = ceil(1,000,000 / 60) = 16,667`. For the Canary's
`canonical-single` scenario and 500 µs margin, the actual configured floors
were:

| Field | Actual configured value |
|---|---:|
| H | 17,167 µs |
| G | 17,167 µs |

The old M3 runner incorrectly reported `configured_H_us` and `expected_hold_us`
as two frames plus margin: **33,834 µs**. This was a reporting error; it did
not change native timing configuration. `HolInterleaved` is the separate
two-frame-hold scenario and retains H = 33,834 µs at these FPS/margin values.

The realized H sample was **29,619.7 µs (n=1)**. Realized G is **unavailable**
because the case contained no subsequent Down. There is not enough data for
native percentiles.

## Sink and event evidence

`AllowSetForegroundWindow` returned false when called for the sink process.
Both actual musical input events nevertheless recorded the ReceiveOnly sink as
the foreground target: HWND `199398`, PID `12548`; the event-level target
HWND/PID matched those foreground HWND/PID values. The sink process was the
ReceiveOnly PowerShell process. The case recorded one Down followed by one Up,
two `SendInput` calls and two inserted key events, with no duplicate, split,
observer-loss, safety-rejection, or unresolved-release-obligation finding.

## Scope and old report wording

This single Canary establishes no A/B performance result, gameplay receipt, or
performance percentile. `SendInput` insertion is not a game receipt.

The old `summary.md`'s generic “Dispatch Core” heading, AB/BA six-pair wording,
and 15-second timed-session wording do not describe this Canary. Its old
`summary.json` and session row carry the erroneous H value above. Read those
fields with this correction applied. The native report's configured minimum
hold/release values were 17,167/17,167 µs; the realized H value is an observed
sample, not the configured floor. No historical field or artifact hash has
been rewritten or recomputed by this erratum.

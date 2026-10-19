# Prepare Whole-Stream Timing Advisory

The whole-stream timing advisory is a bounded, read-only estimate attached to
each native Prepare variant. It describes how the selected authored packet
stream interacts with the configured H/G floors and an explicit illustrative
sender-cost profile. It does not measure Sky input sampling, rendering, or
audio onset, and it never modifies the prepared stream or decides whether a
plan may start.

## Ownership and data flow

Before this change, `sky_player::timing_model::analyze_whole_stream` was used
by model tests and qualification documentation only. Prepare used
`ScheduleMetadata` to construct its Proceed, Use Recommended, and Dry Run
variants, but returned no whole-stream physical floor or head-of-line summary.

Now native Prepare computes an advisory for each variant before publishing
the prepared plan:

```text
authored Song + that variant's effective config and timing policy
  -> ScheduleMetadata
  -> canonical RuntimeSchedule compiler
  -> ordered 15-slot ModelPacket stream
  -> bounded sky_player whole-stream simulator
  -> compact WholeStreamTimingAdvisoryDto on that variant
  -> confirmation UI renders the matching plan fingerprint
```

The compiler remains the authority for packet grouping and order. In
particular, mixed packets retain Up-before-Down masks and one authored
timestamp, stale unmatched Up metadata remains an empty-mask metadata
boundary, and authored relative times are copied without scaling or rebasing.
The advisory is created in the Prepare control path. The dispatch worker,
QPC wait, final Down admission, physical sender, and SendInput suffix do not
call it.

Proceed, Use Recommended, and Dry Run each use their own schedule, effective
config, materialized policy, and plan fingerprint. The recommended schedule
is analyzed only after its recommended tempo/hold policy has been built. The
Dry Run report describes a hypothetical physical projection and is labelled
as such in the UI; Dry Run still emits no gameplay input.

## Timing definitions and assumptions

The adapter maps authored microseconds to relative ticks at 1 MHz, so one
model tick equals one authored microsecond. For the active Prepare policy:

```text
frame_us = ceil(1,000,000 / fps)
H        = ceil(hold_frames * frame_us) + timing_margin_us
authored G = frame_us + timing_margin_us
physical G = frame_us
structural same-key cycle floor = H + authored G
```

The margin is already materialized into H and authored G. The physical
release floor uses one frame and does not add margin again. The floor fields
in the advisory come from the exact variant `MaterializedTimingPolicy`.

The simulator uses this fixed, explicitly illustrative cost profile:

| Assumption | Value |
| --- | ---: |
| QPC model frequency | 1,000,000 Hz |
| Sender setup per physical packet | 10 µs |
| Per-input cost | 3 µs |
| Post-send worker cost | 5 µs |
| Metadata boundary cost | 1 µs |

These values are not claimed as measured p50/p95/p99 sender latency. They are
provided to make a deterministic scenario visible and reproducible. The
model carries a packet's common completion estimate to all keys, applies H
after modeled Down completion and physical G after modeled Up completion,
and includes prior worker occupancy as a possible source of unrelated-key
head-of-line delay. Prepare has no known resume cleanup barrier, so the model
does not invent one.

Negative modeled slack means the selected assumptions place readiness after
an authored packet target. H/G displacement and head-of-line delay counts
identify which modeled boundaries contribute. A zero count means the model
found no pressure for that category under these assumptions; it is not a
delivery guarantee. Every DTO reports `receiver_acknowledged: false`. Sky does
not acknowledge these estimates.

## Bounds, cancellation, and status

The adapter checks its bounds before compiling the model stream or allocating
the analyzer input:

- at most 50,000 authored actions and compiled packets per variant;
- at most 3,000,000 upper-bound key-mask visits (60 visits per packet);
- at most 64 MiB of conservatively estimated additional working storage for
compiler inputs, packet and intent storage, model inputs, estimates, slack
scratch, reason copies, and container overhead.

The Prepare service cooperatively cancels an analysis when a newer Prepare
request supersedes it. The adapter checks cancellation before compilation,
while copying packets, and periodically inside the model loop. All arithmetic
and microsecond-to-tick conversion in the model is checked. If an input is
too large, malformed, cancelled, or overflows, the advisory reports
`unavailable`; the existing Prepare admission result is retained. Invalid or
inconsistent model assumptions report `invalid_assumptions`.

The status enum is additive and version-conscious:

| Status | Meaning |
| --- | --- |
| `not_analyzed` | No result was supplied for this plan, such as an older or incomplete response. |
| `invalid_assumptions` | The configuration, H/G policy, QPC frequency, or checked model arithmetic was invalid. |
| `unavailable` | The bounded computation was skipped or could not complete; the explanation gives the reason. |
| `no_modeled_risk` | The completed estimate found no negative slack, H/G displacement, or HOL delay. |
| `modeled_risk` | At least one completed model category reports pressure under the illustrative assumptions. |

The compact DTO retains counts and maximum modeled displacements, not the
per-packet analyzer vector. Its variant fingerprint must equal the plan
fingerprint beside it. The frontend resolves each Proceed, Recommended, or
Dry Run decision to that exact DTO; absent advisories are shown as unavailable
instead of reusing another variant's estimate.

## Safety and product behavior

Modelled pressure remains explanatory. It does not reject a plan, change
tempo, FPS, hold, margin, packet grouping, or authored timestamps. Existing
impossible same-key repeat validation remains the only relevant authored
feasibility block. The frontend presents advisory details beside each
confirmation choice, and shows the Proceed advisory for a ready plan without
making it a start gate. The frontend describes the estimate as offline and
unacknowledged.

The feature does not change the single sender, one SendInput attempt per
musical packet, H/G completion floors, final Down authority checks, supervisor
lease, panic release, tracked release masks, lifecycle ordering, or terminal
cleanup.

## Regression coverage and limits

Deterministic adapter and UI tests cover empty and sparse streams, dense
alternating keys, mixed packets, 2-key and 15-key chords, supported FPS/hold
profiles and margin edges, near-floor same-key cycles, unrelated-key HOL
pressure, invalid target order, assumption mismatch and overflow, action-limit
skip, cancellation, dry-run caveats, repeatability, unchanged authored actions,
ready-plan modeled pressure, and matching variant fingerprints. Existing
timing-model tests cover checked tick conversion, masks, H/G floors, zero and
negative slack, cancellation, and packet order.

This is an offline Prepare/UI feature. It does not alter sender composition or
physical playback, so ReceiveOnly foreground acceptance and historical
physical A/B campaigns are out of scope. No game receiver or audio timing
claim follows from the model or these tests.

# Real-Time Dispatch Timing Evidence

This contract describes the observations exported by production dispatch and
the separate offline whole-stream timing model. It reports the Windows sender
boundary. It does not observe Sky's key sampling, game scheduling, rendering,
or audio onset.

## Observation sources

Production-normal dispatch records fixed-width timing scalars immediately
after the prepared `SendInput` transaction returns. This evidence is worker
local and does not depend on diagnostic mode, the bounded observer queue, or
telemetry export. The successful final-gate-to-sender suffix remains limited
to its existing atomic admission checks and one prepared send. It adds no
QPC sample, allocation, formatting, logging, I/O, observer wait, or shared
mutex acquisition.

Strict/diagnostic trace records are built by the deferred observer. The
observer queue is bounded and drops new records when full. Both queue drops
and telemetry-ring drops are exported as counts; truncation is explicit. A
disabled trace mode exports no trace rows, while production scalar snapshots
still retain their sample counts and maxima.

The desktop retains at most 4,096 fixed-width trace records per session. This
keeps the raw record storage under the existing 2 MiB budget after adding the
schema 19 provenance and timing fields; all records rejected at that bound
remain reflected in the dropped count.

## Schema 19 and timestamp provenance

Every absolute QPC timestamp in the version 19 native trace has a paired
`*_available` flag and `*_provenance` value. Provenance describes how the
exported field value is obtained: absolute QPC fields are direct samples, and
relative compatibility offsets are derived from a named sampled QPC boundary
and the immutable playback epoch.

| Provenance | Meaning |
| --- | --- |
| `measured` | The exported absolute QPC boundary was directly sampled. |
| `derived_from_wake_qpc` | `wake_offset_ticks` was converted from sampled wake QPC relative to the epoch. |
| `derived_from_pre_call` | The compatibility `send_started_ticks` offset was converted from sampled pre-call QPC relative to the epoch. |
| `derived_from_send_input_completion` | The compatibility `send_completed_ticks` offset was converted from sampled SendInput completion QPC relative to the epoch. |
| `unavailable` | The required source sample or conversion is absent and the numeric field is zero. Absolute QPC fields and `wake_offset_ticks` also have a false availability flag; the deprecated send offsets use this provenance value as their availability signal. |

The absolute QPC fields are wake, target crossing, final policy, sender
pre-call, `SendInput` completion, post-send capture, and observer sample.
Production-normal does not take an extra final-policy QPC sample between its
last lease/control gate and the sender; that field is therefore unavailable
there. Strict or test paths can report it when they actually sample it.

Authored, effective-deadline, and wake offsets are relative to the immutable
playback epoch, not absolute QPC timestamps. Authored schedule QPC targets and
physical floor QPC targets are not serialized as observations. Their
differences are exported with availability derived from the actual required
boundaries. For an unattempted overdue-backlog record, authored lateness uses
the measured observer QPC and the row carries that timestamp as
`observation_qpc`; attempted sends use their sender pre-call sample.

## Derived timing fields

For an attempted physical packet:

```text
authored_lateness = measured pre_call_qpc - authored_target_qpc
floor_displacement = physical_not_before_qpc - authored_target_qpc
residual_lateness = measured pre_call_qpc - physical_not_before_qpc
sendinput_envelope = measured completion_qpc - measured pre_call_qpc
```

Lateness remains signed. A positive authored lateness marks an overdue packet.
Hold, release, and other-floor displacement are reported separately. The
source validates floor subtraction; inconsistent QPC ordering is not silently
clamped into a plausible result.

HOL delay is a lower bound, only when the previous sender completion is known:

```text
hol_lower_bound = min(previous_sendinput_completion, pre_call)
                   - physical_not_before
```

with a zero floor. The unattributed component is `max(0, residual_lateness -
hol_lower_bound)`. It remains unattributed; it is not assigned to an invented
cause. The sender envelope measures only the caller's pre-call-to-return
interval. It does not measure Windows scheduling after return or game receipt.

A transport anomaly is a materialized transaction whose status is not complete,
which reports a Win32 error or skipped inputs, or whose attempt count differs
from one. This includes a rejected transaction that never reached `SendInput`;
control and authority rejections made before transaction creation are not
counted as transport anomalies. This classification does not authorize a
retry.

Trace summaries preserve signed and absolute maxima separately for the new
timing fields, plus per-field sample counts, overdue count, transport
anomalies, and dropped-record counts. Older histogram fields remain in the
schema for compatibility and are not populated by this observation path. The
public engine snapshot exposes fixed-width always-on producer counts and
maxima. Sample counts distinguish absent evidence from a measured zero;
historical absolute schedule QPC values are not included in that snapshot.

## Offline whole-stream risk model

`sky_player::timing_model::analyze_whole_stream` is a pure offline simulator.
It uses caller-supplied sender setup, per-input, post-send, metadata, QPC
frequency, effective hold H, physical release gap T, and optional resume
barrier assumptions. Checked arithmetic rejects invalid masks and overflow.
The public input slice must be chronological by `authored_target_ticks`;
equal targets are allowed, while a decreasing target returns a typed error and
is never silently sorted.
The model preserves packet order, processes Up keys before Down keys in a
mixed packet, tracks per-slot H/T constraints, applies the resume floor to
Down-bearing packets, and carries sequential sender/post-send occupancy into
later packets to expose unrelated-key HOL. It reports signed slack, floor
displacement, modeled lateness, and sender/worker completion estimates.
Positive slack means the authored target is later than all modeled constraints;
zero means equality; negative slack means a floor or prior worker occupancy
pushes readiness past the authored target.

The physical floor recurrence follows packet completion. For physical packet
`i`, let `N_i` be its total number of Up and Down inputs:

```text
R_i = max(authored_target_i, relevant H/T floors, resume barrier for Downs,
          previous_worker_free)
C_i = R_i + setup + N_i * per_input
W_i = C_i + post_send
```

Every Up key in the packet clears its prior H floor and receives
`T_i[key] = C_i + T`; every Down key clears its prior T floor and receives
`H_i[key] = C_i + H`. Floors for keys outside the packet carry forward. All
packet members share `C_i`, matching the production guard's use of one
trustworthy completion QPC for the successful atomic packet. Metadata has no
per-key floors or sender completion; its ready time is the maximum of its
authored target and prior worker-free time, then its configured metadata cost
sets the next worker-free time. Durations are rounded up to QPC ticks and all
additions remain checked.

For a concrete difference, use a 1 MHz QPC frequency, zero setup, 10 ticks per
input, and `H = 10`. A Down chord for A and B at tick 0 completes at tick 20.
The production guard sets both Up floors to tick 30. The old event-ordinal
model assigned A an event completion at tick 10 and predicted its floor at
tick 20, while B received tick 30. An Up for A authored at tick 20 therefore
looked unconstrained in the old report even though the production guard waits
until tick 30. The corrected report uses tick 30 for both chord members.

The estimates remain diagnostic: `C_i` is not an observed `SendInput`
timestamp, and neither sender completion nor this model proves game input or
audio reception. `analyze_whole_stream` currently has no Prepare-path caller.
Attaching it to Prepare would expand `PreparedPlaybackDto` and the desktop
projection/UI contract to describe assumptions and advisory meaning. This
change leaves that as an explicit integration seam; any future presentation
must remain advisory and must not gate Prepare, worker admission, waits, or
musical input.

The result is labeled `guarantee: false`. Estimates never enter production
admission, waiting, or scheduling and cannot rewrite authored targets. Model
cost inputs are estimates rather than machine measurements. Boot constructs a
prepared stream only when both the selected profile and timing configuration
are non-strict. Strict/diagnostic dispatch uses the dynamic planner and its
pending-release recovery branch. That branch remains because the source
selects it and the current tests exercise its coalescing behavior; the
prepared stream has no equivalence proof for it. In particular,
`pending_release_equal_authored_boundary_sends_one_up_packet` verifies that a
pending Up coalesces with an authored Up at the same boundary into one ordered
packet.

## Windows A/B qualification plan

Use the project-owned `ReceiveOnly` sink and the release harness built from
each exact revision. Never target Sky for this qualification. Run the current
candidate and its baseline on the same Windows 11 machine, same power plan,
same display/input setup, and same pinned Rust toolchain. Keep the harness,
runner, sink, and configuration hashes with each result.

For each build, pair diagnostics-on and diagnostics-off runs in randomized
order for quiet, bounded two-worker CPU contention, and minimized-sink
conditions. Include production-normal and strict policy as separate groups,
then repeat the canonical single, chord, mixed Up/Down, and near-minimum
retrigger cases. Record the exact scenario, margin, QPC frequency, source SHA,
binary SHA-256, environment, onset and sender timing distributions, sample and
drop counts, transport status, masks, attempt counts, and cleanup outcome.
Compare paired medians and tails without treating the finite receiver poll as
the exact transition time. Reject the run if ownership, focus, mask/order,
attempt, or cleanup assertions fail.

The current native acceptance runner fixes the production profile and Ring
telemetry mode, so it cannot execute a paired Ring/Off comparison by itself.
The production-normal player regression harness exercises both modes with the
same prepared packet schedule and verifies identical packet masks, order,
attempt count, and timing summaries. Before claiming native A/B evidence, add a
qualification-only mode switch wired to `TelemetryOptions.mode`, keep the
production profile fixed, and run randomized paired cases. Strict policy is a
separate group and is not a substitute for this diagnostics comparison.

The sink is an isolated receiver used to verify the project harness and
receiver observation path. Even a successful native run establishes only
that the harness sent to and the sink observed project-owned test input. It
does not establish that Sky received a transition or that audio began.

## Startup calibration placement qualification

Issue #467 compares the current placement (Baseline A), before scheduling
guards, with Candidate B, after worker preparation allocations, observer
startup, and scheduling-guard acquisition, but before `Ready`, `Armed`, or
epoch publication. The placement selector exists only in test-support and the
feature-gated native qualification harness. Shipping builds have no runtime
placement setting.

Both placements run the same six zero-spin samples with the existing 2 ms
targets and 20 ms startup calibration budget. The absolute deadline remains
anchored at the same QPC sample immediately before Baseline A would probe:

```text
probe_deadline = initial_probe_qpc + 20 ms
```

Candidate B receives only the remaining portion of that deadline. It does not
get a new budget after resource setup. An exhausted deadline selects the same
safe fallback threshold and is recorded as a calibration fallback. The worker
publishes the calibration result and selected threshold before Ready. A
controlled test-only completion seam allows Stop and Skip to race a successful
probe completion without depending on timer resolution; the final pre-Ready
startup gate remains authoritative.

The qualification runner builds one Release executable and alternates AB/BA
within each paired workload cell. It covers sparse single-key sequences,
single-key traffic, mixed Up/Down packets, 15-key chords, and near-minimum
same-key retriggers under quiet and two BelowNormal-priority CPU workers. Each
session writes the bounded native telemetry ring and the exact ReceiveOnly
event window, with checksums and host/build provenance. Raw per-session QPC
records remain under `.benchmarks/wake-calibration-qualification/`; reports
must include per-field availability, missing counts, telemetry truncation,
and empirical p50, p95, p99, p99.9, maximum, and sample count. Low-count cells
must be labeled as weakly supported rather than treating p99.9 as a service
guarantee.

The qualification build enables the existing timing observer through a
test-support-only option while keeping the production dispatch profile and
timing policy. This supplies per-packet QPC traces without using the strict
diagnostic profile or exposing a production setting. A run with no accepted
trace records is invalid for the placement comparison. The harness also
retains the priority/MMCSS mode and power guard acquisition result separately
from their post-session released state.

Before collecting A/B results, Candidate B promotion was conditioned on all
of the following:

- Every paired run passes generation, packet-order, ReceiveOnly event-count,
  joined retirement, and cleanup checks, with zero transport anomalies or
  unresolved release obligations.
- In both load conditions, paired median p99 residual lateness improves by
  at least 10% and 100 µs in at least three of the five workload groups; at
  least five of six pairs in each counted group must move in the improving
  direction. The pooled session p95 startup wake-error metric must also
  improve by at least 10% and 50 µs.
- Candidate Ready-latency p95 increases by no more than 1 ms, no individual
  paired median increases by more than 2 ms, and calibration fallback rate
  increases by no more than one percentage point.
- Worker CPU and process CPU do not increase by more than 5% at the paired
  median, spin duty cycle does not increase by more than two percentage
  points, and acquired priority/MMCSS and power-throttling guard status do not
  regress.

If any required distribution is missing, too small to evaluate, noisy, or
truncated, the evidence is inconclusive and Baseline A remains the shipping
policy. Wake-probe statistics are six startup wait observations per completed
session; they characterize the calibration input rather than production
dispatch p99. Packet timing fields are sampled QPC boundaries around the
sender. ReceiveOnly messages establish sink receipt by this test process only.
The qualification does not measure Sky note reception or audio onset.

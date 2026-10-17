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
schema 18 provenance and timing fields; all records rejected at that bound
remain reflected in the dropped count.

## Schema 18 and timestamp provenance

Every absolute QPC timestamp in the version 18 native trace has a paired
`*_available` flag and `*_provenance` value:

| Provenance | Meaning |
| --- | --- |
| `measured` | The named QPC boundary was actually sampled. |
| `derived_from_pre_call` | The compatibility `send_started_ticks` value was converted from the sampled sender pre-call QPC. |
| `unavailable` | No value for that boundary was sampled. Its numeric field is zero and its availability flag is false. |

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
The model preserves packet order, processes Up keys before Down keys in a
mixed packet, tracks per-slot H/T constraints, applies the resume floor to
Down-bearing packets, and carries sequential sender/post-send occupancy into
later packets to expose unrelated-key HOL. It reports signed slack, floor
displacement, modeled lateness, and sender/worker completion estimates.
Positive slack means the authored target is later than all modeled constraints;
zero means equality; negative slack means a floor or prior worker occupancy
pushes readiness past the authored target.

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

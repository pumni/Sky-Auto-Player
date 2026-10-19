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

Dispatch and observer snapshots have separate owners and are merged only for
readout. A live snapshot publication that cannot acquire its own slot is
skipped without making the other source's counters appear to be its own. A
zero wait maximum with `wait_window_sample_count == 0` means no deadline-wake
measurement was recorded; it is not a measured zero-lateness wake. Interrupted
waits retain their QPC/deadline attribution but do not enter the deadline-wake
health sample population.

For a deadline wait, the wake QPC is captured from the wait result and copied
into worker-owned pending evidence as soon as control returns from the wait.
The corresponding bounded observer-queue insertion occurs only after that
dispatch attempt returns, so queue insertion time is not wake time and does not
replace the captured wake timestamp. Interrupted waits have no following
dispatch attempt and are inserted on their immediate replan path.

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
audio reception. Native Prepare now calls the simulator through the bounded
variant adapter documented in
[`prepare-whole-stream-timing-advisory.md`](prepare-whole-stream-timing-advisory.md).
That adapter limits action/packet count, work, and estimated working storage
before constructing the model vector; it returns only compact summary fields
on each variant DTO. It does not add analyzer work to dispatch, waits, final
Down admission, or sender delivery.

The result is labeled `guarantee: false`. Estimates never gate production
admission or alter waiting/scheduling and cannot rewrite authored targets.
Model cost inputs are illustrative assumptions rather than machine
measurements. Boot constructs a prepared stream only when both the selected
profile and timing configuration are non-strict. Strict/diagnostic dispatch
uses the dynamic planner and its pending-release recovery branch. That branch
remains because the source selects it and the current tests exercise its
coalescing behavior; the prepared stream has no equivalence proof for it. In
particular, `pending_release_equal_authored_boundary_sends_one_up_packet`
verifies that a pending Up coalesces with an authored Up at the same boundary
into one ordered packet.

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

### Windows 11 paired results — 2026-10-18

The promotion gate was applied to qualification `wake-calibration-20261018T161010-7459e9f8`.
The measured source revision was
`a9d74c2e5ff919c701b4797d194ca29efa9f7d47`, with a clean source tree. Both
placements used the same Release harness binary
(`6890349d508ed753d690bd51ca075583789ccde672804210ce96ba10e71b1d12`). The
platform was Windows 11 x64 with Rust `1.99.1 (58cf8f9c3 2026-10-13)` and
10,000,000 QPC ticks per second. Timing margin was 500 µs. The committed
evidence omits the host CPU model, exact Windows build, power-scheme identifier,
process identifiers, absolute paths, and raw event timestamps.

The runner alternated AB and BA order across 60 pairs: six repetitions for
each of five workloads under quiet conditions and under two hidden
BelowNormal-priority CPU workers. The workloads were
`long-single-sequence`, `canonical-single`, `mixed-up-down`,
`canonical-max-chord` (15-key chord), and `near-minimum-retrigger`. It ran 120
sessions total, with 60 sessions per placement. Every session passed the
ReceiveOnly acceptance checks. Each placement completed all six calibration
samples in all 60 sessions, with zero calibration fallbacks. MMCSS `Games`
and the startup power guard were acquired in all sessions. There were no
transport anomalies, unresolved release obligations, failed release checks,
or nonzero final release masks. The telemetry ring accepted 756 records for
each placement; both placements had zero telemetry drops, observer queue
drops, and truncated reports.

The tables use empirical nearest-rank quantiles. Session rows summarize 60
session values per placement. Dispatch rows summarize 756 packet records,
except HOL, which has 696 available values and 60 expected missing first-packet
values. `p99.9` equals the maximum at these sample counts and is not a tail
service guarantee. Values are microseconds unless the unit column says
otherwise; counts show `samples / missing`.

| Session metric | Unit | Baseline A p50 / p95 / p99 / p99.9 / max | A n / missing | Candidate B p50 / p95 / p99 / p99.9 / max | B n / missing |
| --- | --- | --- | --- | --- | --- | --- |
| Ready latency | µs | 14667 / 15328 / 15495 / 15495 / 15495 | 60 / 0 | 13869 / 14627 / 14794 / 14794 / 14794 | 60 / 0 |
| Calibration duration | µs | 14272 / 14919 / 15074 / 15074 / 15074 | 60 / 0 | 13477 / 14207 / 14362 / 14362 / 14362 | 60 / 0 |
| Startup wake error, per-session p50 | µs | 399 / 512 / 516 / 516 / 516 | 60 / 0 | 80 / 451 / 512 / 512 / 512 | 60 / 0 |
| Startup wake error, per-session p95 | µs | 519 / 669 / 831 / 831 / 831 | 60 / 0 | 530 / 853 / 954 / 954 / 954 | 60 / 0 |
| Effective spin threshold | µs | 838 / 1000 / 1000 / 1000 / 1000 | 60 / 0 | 911 / 1000 / 1000 / 1000 / 1000 | 60 / 0 |
| Worker CPU time | µs | 0 / 31250 / 62500 / 62500 / 62500 | 60 / 0 | 0 / 62500 / 78125 / 78125 / 78125 | 60 / 0 |
| Process CPU time | µs | 0 / 46875 / 78125 / 78125 / 78125 | 60 / 0 | 0 / 62500 / 78125 / 78125 / 78125 | 60 / 0 |
| Spin duty cycle | ppm | 0 / 0 / 0 / 0 / 0 | 60 / 0 | 0 / 0 / 0 / 0 / 0 | 60 / 0 |

| Dispatch metric | Unit | Baseline A p50 / p95 / p99 / p99.9 / max | A n / missing | Candidate B p50 / p95 / p99 / p99.9 / max | B n / missing |
| --- | --- | --- | --- | --- | --- | --- |
| Authored lateness | µs | 28.9 / 173.5 / 329.7 / 612.8 / 612.8 | 756 / 0 | 29.5 / 206.2 / 515.0 / 720.7 / 720.7 | 756 / 0 |
| Physical floor displacement | µs | 0 / 170.6 / 322.2 / 611.1 / 611.1 | 756 / 0 | 0 / 181.1 / 462.9 / 605.2 / 605.2 | 756 / 0 |
| Residual lateness | µs | 8.9 / 28.6 / 56.9 / 248.7 / 248.7 | 756 / 0 | 9.1 / 34.9 / 173.7 / 291.6 / 291.6 | 756 / 0 |
| HOL lower bound | µs | 0 / 0 / 0 / 0 / 0 | 696 / 60 | 0 / 0 / 0 / 0 / 0 | 696 / 60 |
| Sender envelope | µs | 128.6 / 341.8 / 490.7 / 2123.3 / 2123.3 | 756 / 0 | 129.0 / 266.0 / 560.6 / 819.9 / 819.9 | 756 / 0 |

The per-session startup wake p95 distribution moved from p50/p95
`519/669 µs` under Baseline A to `530/853 µs` under Candidate B. The packet
residual-lateness p99 moved from `56.9 µs` to `173.7 µs`. Ready latency p95
was 701 µs lower for B, with the largest paired Ready increase 561 µs. The
Ready result did not offset the wake and dispatch regressions.

Paired p99 residual-lateness deltas below are `Candidate B - Baseline A` in
µs; a negative value favors B. No group met the required 100 µs and 10%
improvement in at least five of six pairs.

| Workload | Load | Median paired p99 delta | Pairs improved / 6 |
| --- | --- | ---: | ---: |
| 15-key chord | Quiet | -0.3 | 4 |
| 15-key chord | Contention | -1.2 | 3 |
| Canonical single | Quiet | -2.1 | 4 |
| Canonical single | Contention | -2.5 | 3 |
| Long single sequence | Quiet | -8.1 | 4 |
| Long single sequence | Contention | -4.4 | 3 |
| Mixed Up/Down | Quiet | -2.6 | 6 |
| Mixed Up/Down | Contention | -0.4 | 3 |
| Near-minimum retrigger | Quiet | +2.5 | 1 |
| Near-minimum retrigger | Contention | -0.4 | 3 |

Windows CPU time counters were quantized in 15.625 ms steps. Their zero
medians and coarse upper quantiles cannot establish the 5% CPU-cost gate; the
long-single quiet worker median was one counter step higher for B, so the
available CPU evidence does not support promotion. Reported zero spin duty
cycle is the recorded value and does not establish that no spin occurred.
Candidate B therefore fails the predeclared improvement gate and the shipping
recommendation is to retain Baseline A. Candidate placement remains
qualification-only; production continues to calibrate before scheduling
guards.

Timing attribution is limited to measured QPC boundaries: calibration sample
wake errors, authored and floor-relative packet timing against the sampled
sender pre-call boundary, and the pre-call-to-`SendInput` completion envelope.
The OS worker/process CPU counters are separate coarse CPU-time observations.
The dedicated ReceiveOnly sink verified event delivery from the test harness.
No metric here establishes a Sky key transition, note reception, or audio
latency.

Reproduce on a Windows 11 x64 host with the pinned toolchain using:

```powershell
pwsh.exe -NoProfile -ExecutionPolicy Bypass -File scripts/qualify_startup_calibration.ps1 -TimingMarginUs 500
```

The qualification build command was `cargo build --locked --manifest-path
rust/Cargo.toml -p sky_player --features real-input-acceptance --bin
rt-native-acceptance --release` (exit 0). The run summary and ordered session
ledger are retained locally under
`.benchmarks/wake-calibration-qualification/wake-calibration-20261018T161010-7459e9f8/`;
the 120 per-session reports, sink event windows, invocation details, and
checksums are under `.benchmarks/physical-native-cases/`. The summary SHA-256
is `4036a7c77ffa5f038de44ed284c177dc2f0ff3c7c0a360e541a59d425fd92cd2`, the
ordered ledger SHA-256 is
`0969ed103082412ed810f07d338ef8ab901e584b7e4540f189c6e15df3d7e767`, and the
qualification metadata SHA-256 is
`9f49decd7987addae29e8487b10aafa36f0eaf9acac234f87e0a88733e467e21`.

The sanitized, machine-readable projection is
[`wake-calibration-qualification-20261018.json`](evidence/wake-calibration-qualification-20261018.json),
SHA-256 `68323b15e8f47773f92fda79187f9dfe61e3e673898fd4a8af256ec49a8070a8`.
It contains the 120-row run-order ledger,
60 paired comparisons, all 20 workload/load/placement cells, nearest-rank
distributions, per-field sample/missing/drop/truncation counts, and artifact
hashes. `scripts/export_startup_calibration_evidence.ps1` verifies the
qualification manifest and every file listed by all 120 per-session
manifests before it exports the projection. The accepted local artifact set
passed that verification: four qualification inputs and all files in 120 case
directories (1,320 files total); the qualification manifest SHA-256 is
`c26829d68a5c744392dca4f9b38bd70b8ff8fb1d4485851772a26a229816c6b7`. Recreate
the sanitized projection from the preserved local artifact directories with:

```powershell
pwsh.exe -NoProfile -ExecutionPolicy Bypass -File scripts/export_startup_calibration_evidence.ps1 -QualificationDirectory .benchmarks/wake-calibration-qualification/wake-calibration-20261018T161010-7459e9f8 -CaseRoot .benchmarks/physical-native-cases -OutputPath docs/evidence/wake-calibration-qualification-20261018.json
```

The raw reports and sink traces remain ignored under
`.benchmarks/`; the committed projection preserves their relevant checksums
without publishing the raw records or local machine identifiers.

The measured source SHA above differs from the final PR head. Changes after the
measured source only affect diagnostic reporting, completion-handle retirement,
test support, source organization, or the acceptance harness's handling of an
already-foreground ReceiveOnly sink. Baseline A remains the production
calibration placement. The join correction runs after worker terminal cleanup
and does not change authored QPC deadlines, calibration placement, timing
floors, dispatch decisions, or SendInput calls. The focus helper itself is
restored to its earlier strict contract; the acceptance harness now checks the
exact already-foreground sink HWND before requesting a focus change. The
recorded 120-session timing comparison therefore remains applicable without
repeating the experiment.

Three earlier attempts were excluded from the comparison: one lacked accepted
timing-observer records, one lacked retained scheduling-guard acquisition
evidence, and a partial run stopped after focus verification falsely rejected
an already-foreground sink. The latter case remained pre-arm and sent no
input. The accepted 120-session run used the corrected harness and passed all
acceptance checks.

## Prepared commit pre-wait experiment for issue #475

The proposed change was to validate a frozen prepared authored commit before
the physical wait, then carry an iteration-scoped proof across the wait. The
candidate proof borrowed both the exact immutable stream entry and coordinator
so their cursor and cancellation ownership could not change during the wait;
replans discarded it, and all post-wake physical admission checks remained
fresh. The candidate was safe under the exercised checks, but the Windows
latency evidence did not demonstrate a repeatable tail improvement. The
runtime change was therefore not promoted: current code continues to validate
the frozen coordinator commit after each due wait and before prepared physical
admission.

Baseline A was clean `fcb610bbc13ee25b852eab3b2be880e07379cff2`; candidate B
was clean `a00d531582311b2232e9db84ccb143f1964e4b21`. Both acceptance binaries
were built with `cargo build --locked --manifest-path rust/Cargo.toml -p
sky_player --release --features real-input-acceptance --bin
rt-native-acceptance`. A's executable SHA-256 was
`919db9392a43b4373f692f41dcf47121741c693d5d82e6b6c8dee78ea5795403`; B's was
`ad548bb632fd4f3f78ed142b47aab9dca8c87448f36efd564d355b4fcc45cb7d`. Both
source trees were clean during measurement. Runs alternated A/B order by pair,
used the same production ReceiveOnly sink, 500 µs margin, and timing observer.
The environment was Windows 11 Pro build 26300, AMD Ryzen 5 5600 (6 cores, 12
logical processors), Balanced power plan, and `rustc 1.99.1 (58cf8f9c3
2026-10-13)`. QPC frequency was 10 MHz. These tests measured the Windows
SendInput-to-sink boundary; they did not measure game sampling or musical
output.

Run commands, invoked once per session by the alternating qualification
runner, were:

```powershell
pwsh.exe -NoProfile -File scripts/run_native_acceptance_case.ps1 -Scenario dense-alternating -TimingMarginUs 500 -TimingObserverForQualification
pwsh.exe -NoProfile -File scripts/run_native_acceptance_case.ps1 -Scenario mixed-up-down -TimingMarginUs 500 -TimingObserverForQualification
```

The dense workload used 20 sessions per build and 640 attempted/accepted
packets per build. The mixed Up/Down workload used 60 sessions per build and
180 attempted/accepted packets per build. Wake-to-precall latency is nearest
rank p50/p95/p99/max in microseconds, measured as
`pre_call_qpc - wake_qpc`. Fourteen dense baseline packets had no wake sample
because the target was already due; all 640 candidate dense packets and all
180 mixed packets per build had wake samples. These were unavailable wake
observations, not telemetry drops.

| Workload | Build | Wake-to-precall p50 / p95 / p99 / max (µs) | SendInput envelope p50 / p95 / p99 / max (µs) | Core post-send p50 / p95 / max (µs) | Process CPU p50 / p95 (ms) |
| --- | --- | ---: | ---: | ---: | ---: |
| Dense alternating | Baseline A | 3.1 / 11.3 / 30.7 / 584,577.5 | 97.8 / 170.8 / 576.6 / 157,337.7 | 1 / 3 / 8 | 31.25 / 46.875 |
| Dense alternating | Candidate B | 2.4 / 11.2 / 41.0 / 165.9 | 98.4 / 173.1 / 520.3 / 783.1 | 2 / 3 / 10 | 15.625 / 46.875 |
| Mixed Up/Down | Baseline A | 11.2 / 30.7 / 1,413.6 / 1,656.0 | 129.7 / 158.1 / 226.7 / 237.4 | 2 / 5 / 6 | 0 / 15.625 |
| Mixed Up/Down | Candidate B | 10.9 / 49.2 / 1,683.7 / 2,439.7 | 128.4 / 198.7 / 225.7 / 234.0 | 3 / 6 / 7 | 0 / 15.625 |

Paired per-session median wake-to-precall deltas (B minus A) were -0.3 µs at
the dense median and -0.4 µs at the mixed median. The candidate's median was
lower in 11 of 20 dense pairs and 34 of 60 mixed pairs. The p95 paired-session
delta was +0.2 µs dense and +13.9 µs mixed; the candidate's pooled mixed p95
and p99 were also higher. The very large dense baseline maxima coincide with
extreme wake-to-precall and SendInput-envelope samples. The harness cannot
attribute those intervals to `SendInput` execution versus preemption around a
sampled boundary, so they are reported and not trimmed. Candidate B therefore
did not establish a repeatable improvement in tail behavior, despite its small
pooled median reduction.

All 160 measured sessions passed the sink acceptance checks. Both builds had
zero hold-floor or release-floor violations, transport anomalies, timing
arithmetic anomalies, production forensics anomalies, telemetry drops,
observer-queue drops, or nonzero terminal ownership/failed-release state. The
process CPU field is quantized at approximately 15.625 ms on this host; dense
CPU sums were 437.5 ms for A and 421.875 ms for B across 20 sessions, and mixed
CPU sums were 156.25 ms for each build across 60 sessions. This resolution is
too coarse to attribute a small per-packet CPU change.

The existing after-send work was also reviewed. Wait evidence insertion uses a
fixed-capacity drop-new queue. Live packet snapshots use a 50 ms cadence and a
nonblocking `try_write`; unavailable snapshot publication is already
best-effort. Backend-counter refreshes are scalar copies and the measured core
post-send samples remained in the 1–10 µs range. The A/B data does not identify
these bounded operations as a dominant source of late calls, so no additional
publication machinery or timing-path change was added. The raw per-session
reports and alternating ledgers remain under the local ignored
`.benchmarks/dispatch-prewait-qualification/` and
`.benchmarks/physical-native-cases/` directories. The dense ledger SHA-256 is
`63a744afc7a9ebe0ccc755a8a009da3537ff61c8e17b337a6e0229477815f9f2`; the
mixed ledger SHA-256 is
`0bd7b963ef5ca85805e0f821bf27238fc8cf9b458699a4ec31f704f489eb7605`.

# Dispatch forensics cost qualification (#476)

## Decision

Retain the current production design. This package changes tests and documentation only; it does
not change sender behavior, physical timing, cleanup, `SendInput`, or UI timing policy. Release
component measurements put successful-packet forensics in the tens of nanoseconds, and the
15-key workload remains below 30 ns at batch-mean p99. Removing the independent H/G oracle would
discard separately measured physical evidence without a demonstrated dispatch benefit.

The initial qualification code commit is `8d9d30a73f41ffa2ff5be6900fdb9a41211f74bf`, directly
based on the assigned baseline `8e965ca626e8657ac4e9433fe46bc7456e102fde`. Test isolation and the
Clippy correction are in `c16562c32e2a1f10ec67a68ab34599a8da3b9a0e`. Across those commits, the
Rust source changes are test-only qualification modules; the desktop source change adds assertions
to an existing serialization test.

The fixed anomaly ring is also retained. It is bounded to 32 fixed-width records (1,328 bytes on
the measured x64 target), is written only when an anomaly is classified, and does not participate
in normal-packet output. Its individual entries are intentionally not part of the current public
DTO; public consumers retain aggregate anomaly counts. The old expiration counters are compatibility
fields rather than active admission state and remain serialized as before.

## Ownership and cost map

| Component | Responsibility and evidence | Runtime work | Consumers and decision |
|---|---|---|---|
| `PhysicalTimingGuard` | Admission oracle for per-key physical hold and release floors plus the lifecycle resume Down barrier. It uses successful sender completion timestamps and fails closed when invalidated. | Fixed two 15-slot `Option<QpcTicks>` arrays, one optional resume barrier, and bounded mask-bit walks. Query and update do not sample QPC, allocate, lock, read settings, or walk authored work. | Production dispatch uses its answer to delay packet admission. Keep; this is safety state, not duplicate reporting. |
| `ProductionHoldForensics` | Independent post-send check of Down-completion-to-Up-start H and Up-completion-to-next-Down-start G. It also counts structural anomalies, unmatched Ups, anchor overwrites, and same-key overlap evidence. It observes only fully successful sends when transport progress is uncertain. | At most 15 Up and 15 Down bit iterations per packet, checked timestamp differences, scalar counter/minimum updates, and ring writes only on anomalies. It reuses sender timestamps and adds no QPC reads, allocations, locks, or schedule walk. | Worker metrics feed `EngineSnapshot`, `WorkerMetricsSnapshot`, and native acceptance JSON. Keep because it independently checks actual sender timestamps against the guard's admission decision. |
| Anomaly ring | Preserves the most recent 32 detailed anomaly records and counts overwrites. | 32 fixed records, 32 validity bits, and ring indices; 1,328 bytes total on the tested x64 target. No normal-packet ring writes. | Aggregate counters are public in the engine snapshot and acceptance report. Individual rows have a test-only reader today; retain the bounded detail while the independent forensic path is present. |
| Backend counter refresh | Copies tracked backend counts and error-state counters into local worker metrics. | Scalar copies; the separate error string lock occurs only when the backend error state changes. | Best-effort/live snapshot and lifecycle metrics. Not timing-policy evidence and not a candidate for replacing the physical guard. |
| Live snapshot publication | Makes current metrics visible to observers. | Packet publication is cadence-limited and uses `try_write`; contention skips that best-effort update rather than blocking dispatch. Lifecycle publication follows its own required path outside the final precision suffix. | Live diagnostics and UI snapshot consumers. Retain existing bounded/drop behavior. |
| Pending wait observations | Captures wait outcome and wake QPC for the optional observer. | The wake result is copied before control transitions; queue insertion follows dispatch, except interrupted replans. The fixed-capacity queue is drop-new and increments drop/high-watermark counters. | Observer queue and trace telemetry, not the production H/G proof. Keep separate from safety state. |
| Observer telemetry collector | Stores packet/wait trace records when observer mode is enabled. | Bounded ring storage and record conversion in observer processing; it may drop when bounded capacity is exhausted. | Acceptance telemetry and optional diagnostic trace consumers. It does not replace production scalar forensics. |

### Deprecated and compatibility state

`final_sender_window_expirations`, `prepared_normal_sender_expirations`,
`missed_physical_window_boundaries`, and `release_floor_infeasible_boundaries` had no production
writers and do not control execution. Their dead `WorkerMetricsLocal` storage and test accessors
have been removed. The first, third, and fourth fields remain in public Rust snapshots and the
desktop diagnostics DTO with their existing names, types, order, and numeric zero projection; the
prepared-normal sender counter had no public field and has no replacement storage. Golden
serialization coverage keeps the desktop keys and zero values stable. Active physical timing
outcomes are represented by floor-delay and final-gate metrics instead. See
[`native-playback-ownership.md`](native-playback-ownership.md) for the producer/storage/reader/JSON
matrix.

## Deterministic invariant coverage

The added test matrix exercises dense alternating packets, sparse single-key packets, mixed Up/Down
packets, chord sizes 2 through 15, and same-key retriggers. It compares guard queries with the
independent forensic observations from fully successful sends and asserts zero H/G violations.
Additional tests cover lifecycle reset and resume barriers, partial transport uncertainty, wait
interruption/replan anchor preservation, anomaly-ring capacity/overwrite behavior, and serialized
desktop compatibility fields. These tests characterize unchanged production code; they do not add
new runtime instrumentation.

The timing-model test calls `sky_player::timing_model::analyze_whole_stream` separately from native
acceptance. It uses H=G=17,167 µs at a synthetic 1 MHz QPC rate and explicitly illustrative sender
costs of 10 µs setup, 3 µs per input, and 5 µs post-send. These are model inputs, not measurements or
guarantees about Windows scheduling, `SendInput`, Sky, or a game receiver.

| Offline workload | Packets | Negative-slack packets | Max H displacement (µs) | Max G displacement (µs) | Max modeled HOL lower bound (µs) | Max modeled lateness (µs) |
|---|---:|---:|---:|---:|---:|---:|
| Dense alternating | 16 | 8 | 13 | 0 | 0 | 13 |
| Sparse single key | 48 | 24 | 13 | 0 | 0 | 13 |
| Mixed Up/Down | 10 | 9 | 141 | 125 | 0 | 141 |
| 2–15-key chord sweep | 28 | 14 | 55 | 0 | 0 | 55 |
| Same-key retrigger | 8 | 7 | 91 | 78 | 0 | 91 |
| Separate 14-key HOL stress | 4 | — | — | — | 56 | — |

The separate stress stream places a 14-key Down packet immediately before the fifteenth key's
Down. Its 56 µs modeled head-of-line lower bound is an offline feasibility result only.

## Release component-cost measurement

The ignored, Windows-only `dispatch_cost_qualification` test isolates existing component calls
after constructing samples. It uses `Instant` around each component batch and is not inserted into
the QPC-to-`SendInput` suffix. For each workload and component, it records 512 batch means, each
covering 512–532 packet operations. The table reports the nearest-rank p50/p95/p99 and maximum of
those **batch means**, in ns per packet operation; these are not per-call latency percentiles.
There were 262,144 dense and retrigger operations, 270,336 sparse operations, 266,240 mixed
operations, and 272,384 chord-sweep operations per component. The test verifies zero H/G violations,
zero observer queue drops, and zero telemetry ring drops for its synthetic accepted samples.

| Workload | Component | p50 | p95 | p99 | Max |
|---|---|---:|---:|---:|---:|
| Dense alternating | Production forensics | 10.55 | 10.94 | 11.91 | 19.14 |
| Dense alternating | Guard query + update | 8.20 | 8.40 | 9.96 | 14.45 |
| Dense alternating | Backend counter refresh | 3.91 | 4.10 | 5.08 | 8.01 |
| Dense alternating | Best-effort snapshot publication | 4.88 | 5.08 | 5.27 | 43.16 |
| Dense alternating | Pending wait capture + queue | 31.25 | 32.42 | 39.65 | 62.30 |
| Dense alternating | Telemetry record + ring | 55.27 | 104.10 | 108.98 | 161.72 |
| Sparse single key | Production forensics | 10.42 | 10.61 | 11.17 | 17.61 |
| Sparse single key | Guard query + update | 8.14 | 8.33 | 8.52 | 13.45 |
| Sparse single key | Backend counter refresh | 3.98 | 3.98 | 4.73 | 11.55 |
| Sparse single key | Best-effort snapshot publication | 20.83 | 21.02 | 24.05 | 39.77 |
| Sparse single key | Pending wait capture + queue | 31.25 | 31.82 | 39.20 | 80.68 |
| Sparse single key | Telemetry record + ring | 101.52 | 111.55 | 154.55 | 215.15 |
| Mixed Up/Down | Production forensics | 9.62 | 9.81 | 10.77 | 34.23 |
| Mixed Up/Down | Guard query + update | 8.65 | 8.85 | 9.42 | 18.46 |
| Mixed Up/Down | Backend counter refresh | 4.04 | 4.04 | 4.62 | 6.73 |
| Mixed Up/Down | Best-effort snapshot publication | 6.15 | 6.54 | 8.27 | 11.15 |
| Mixed Up/Down | Pending wait capture + queue | 31.35 | 32.50 | 38.85 | 55.58 |
| Mixed Up/Down | Telemetry record + ring | 55.38 | 82.31 | 105.58 | 136.54 |
| 2–15-key chord sweep | Production forensics | 19.36 | 19.55 | 25.00 | 32.52 |
| 2–15-key chord sweep | Guard query + update | 22.74 | 22.93 | 23.12 | 31.95 |
| 2–15-key chord sweep | Backend counter refresh | 3.95 | 4.14 | 4.51 | 7.33 |
| 2–15-key chord sweep | Best-effort snapshot publication | 20.68 | 21.05 | 23.68 | 37.03 |
| 2–15-key chord sweep | Pending wait capture + queue | 31.20 | 31.77 | 37.59 | 77.82 |
| 2–15-key chord sweep | Telemetry record + ring | 55.45 | 103.01 | 139.85 | 167.29 |
| Same-key retrigger | Production forensics | 10.35 | 10.55 | 12.50 | 20.12 |
| Same-key retrigger | Guard query + update | 8.01 | 8.40 | 8.59 | 11.52 |
| Same-key retrigger | Backend counter refresh | 3.91 | 4.10 | 4.88 | 5.86 |
| Same-key retrigger | Best-effort snapshot publication | 7.03 | 7.42 | 8.59 | 12.50 |
| Same-key retrigger | Pending wait capture + queue | 31.25 | 31.64 | 37.70 | 40.43 |
| Same-key retrigger | Telemetry record + ring | 55.27 | 83.79 | 118.55 | 141.60 |

These component timings are local release-build microbenchmarks. They are not paired against a
runtime candidate because no production runtime code changed. They are not native per-call samples,
and they do not establish game-receiver behavior. This cost profile does not justify deleting the
independent forensics oracle or adding publication machinery.

## Windows receive-only native qualification

Native sessions use the project-owned receive-only sink and production acceptance harness. They
verify exact sink event-window identity and report sender-side wake, pre-call, completion, H/G floor,
anomaly, queue-drop, and ownership evidence. Sink observations establish that the owned test window
received the expected sequence; they are not measurements of Sky or another game receiver.

The local Windows host was Windows 11 Pro build 26300 on an AMD Ryzen 5 5600 (6 cores, 12 logical
processors), using `rustc 1.99.1 (58cf8f9c3 2026-10-13)`. Native reports use a 10 MHz QPC frequency,
production profile, default power plan, no synthetic CPU contention, and the startup-selected
`mmcss:Games` priority mode. These results are local-only measurements.

The 224-session distribution matrix used harness SHA-256
`65329c4222fa0bd5910f8e16fb038e3565a107fd2ab9d5f9eb15dff952845aae`, built from the exact
assigned baseline source `8e965ca626e8657ac4e9433fe46bc7456e102fde`. Its runner checkout was the
qualification commit `8d9d30a73f41ffa2ff5be6900fdb9a41211f74bf`; the intervening diff contains
tests only, and the release acceptance feature does not compile those test-only changes. A later
final post-commit build at `c16562c32e2a1f10ec67a68ab34599a8da3b9a0e` produced candidate harness
SHA-256 `02bd5c21f8806522aebceda6a8968707edc61793243dc1cde55f8b55905e3a35`. Eight additional
one-session checks on that exact candidate binary passed for the five representative workloads and
the three lifecycle cases listed below. These are behavior checks, not a paired performance
comparison.

The tracked [native session ledger](dispatch-forensics-native-sessions.csv) records per-session
identities, counts, tail summaries, and raw report hashes. Its SHA-256 is
`9a85b507b3e27b7027b04060eb6429bbbfd3d6e539aae559dc084620dac39c3e`. The detailed native runner
artifacts remain local under the ignored `.benchmarks/physical-native-cases/` directory; their
individual hashes are in the ledger.

| Workload | Sessions | Expected sink key events | Expected timing records | Accepted timing samples | Sink failures | H/G violations | Forensic/transport anomalies | Telemetry or observer drops | CPU resolution |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| Dense alternating | 20 | 640 | 640 | 640 | 0 | 0 | 0 | 0 | 15.625 ms |
| Sparse single key | 20 | 960 | 960 | 960 | 0 | 0 | 0 | 0 | 15.625 ms |
| Mixed Up/Down | 100 | 400 | 300 | 300 | 0 | 0 | 0 | 0 | 15.625 ms |
| 2–15-key chord sweep | 20 | 4,760 | 560 | 560 | 0 | 0 | 0 | 0 | 15.625 ms |
| Same-key retrigger | 64 | 512 | 512 | 512 | 0 | 0 | 0 | 0 | 15.625 ms |

Every expected sink key event was observed in order. H/G sample counts were 3,636 / 3,012 across
the matrix, with zero hold-floor or release-floor violations. Production forensics, anomaly ring
overwrite, transport, timing arithmetic, partial-send, zero-progress, telemetry-drop, and observer
queue-drop counters were all zero. All sessions ended with zero active, possibly active, failed
release, and stuck keys; terminal release verification succeeded.

The first 30 mixed-workload reports were excluded because the docs index was being edited and the
runner marked the tracked checkout dirty. They all passed sink acceptance, but were not pooled.
Thirty clean replacement sessions were added, leaving 100 clean mixed sessions in the ledger.

The following table pools per-packet observations from those 224 clean sessions. Percentiles use
nearest rank and include the maximum; no tail observations were trimmed. Sender envelope and
residual lateness use existing QPC tick fields converted at the run-reported 10 MHz frequency.
Wake-to-pre-call is computed offline from existing wake and pre-call samples. Core post-send is the
existing captured metric. No timestamps or other instrumentation were added to the QPC-to-`SendInput`
suffix. All four sources had complete availability for every accepted timing record.

| Workload | Measure (µs) | n | p50 | p95 | p99 | Max |
|---|---|---:|---:|---:|---:|---:|
| Dense alternating | Sender envelope | 640 | 113.80 | 175.50 | 585.70 | 809.10 |
| Dense alternating | Wake to pre-call | 640 | 4.00 | 12.90 | 29.60 | 57.40 |
| Dense alternating | Core post-send | 640 | 2.00 | 3.00 | 6.00 | 6.00 |
| Dense alternating | Residual lateness | 640 | 8.40 | 13.40 | 31.10 | 173.10 |
| Sparse single key | Sender envelope | 960 | 119.40 | 189.10 | 539.80 | 850.80 |
| Sparse single key | Wake to pre-call | 960 | 4.40 | 13.00 | 32.30 | 180.90 |
| Sparse single key | Core post-send | 960 | 2.00 | 3.00 | 6.00 | 23.00 |
| Sparse single key | Residual lateness | 960 | 7.60 | 13.50 | 41.90 | 180.90 |
| Mixed Up/Down | Sender envelope | 300 | 136.10 | 185.30 | 229.70 | 304.70 |
| Mixed Up/Down | Wake to pre-call | 300 | 11.40 | 33.10 | 226.00 | 993.40 |
| Mixed Up/Down | Core post-send | 300 | 2.00 | 6.00 | 9.00 | 10.00 |
| Mixed Up/Down | Residual lateness | 300 | 11.40 | 33.30 | 226.00 | 993.40 |
| 2–15-key chord sweep | Sender envelope | 560 | 279.40 | 481.00 | 939.70 | 2,638.80 |
| 2–15-key chord sweep | Wake to pre-call | 560 | 6.10 | 14.80 | 33.10 | 50.50 |
| 2–15-key chord sweep | Core post-send | 560 | 2.00 | 4.00 | 6.00 | 15.00 |
| 2–15-key chord sweep | Residual lateness | 560 | 9.70 | 22.10 | 46.50 | 276.90 |
| Same-key retrigger | Sender envelope | 512 | 123.70 | 244.50 | 411.10 | 935.40 |
| Same-key retrigger | Wake to pre-call | 512 | 4.40 | 30.60 | 70.50 | 835.80 |
| Same-key retrigger | Core post-send | 512 | 2.00 | 6.00 | 6.00 | 9.00 |
| Same-key retrigger | Residual lateness | 512 | 8.70 | 30.60 | 123.60 | 835.80 |

The largest sender-envelope sample was 2,638.80 µs in the chord sweep, and the largest wake-to-
pre-call sample was 993.40 µs in the mixed workload. These tails are retained in the reported
maxima. No claim is made about game receiver or musical onset timing.

Process / worker CPU totals across the sessions were: dense 500.000 / 359.375 ms, sparse 781.250 /
656.250 ms, mixed 343.750 / 109.375 ms, chord 703.125 / 390.625 ms, and retrigger 453.125 /
312.500 ms. These CPU values include session setup and are quantized in 15.625 ms increments; they
cannot resolve a per-packet or nanosecond-scale component cost.

The candidate-harness smoke set passed one session each for dense (32 sink events), sparse (48),
mixed (4), chord sweep (238), retrigger (8), full-release cleanup (30), stop cleanup (2), and
pause/resume release-floor (42). H/G violations, anomalies, drops, and terminal ownership failures
were zero in all eight. An earlier baseline-harness pause/resume attempt was `INCONCLUSIVE` before
arming because Windows did not grant the receive-only sink foreground status; it emitted no
physical packets. The same scenario passed once in a separate invocation on the unchanged baseline
harness and again on the candidate harness. The inconclusive report remains in the local raw
artifacts and is not counted as a passing run.

### Reproduction

Component measurement:

```powershell
rtk bypass cargo test --release --locked --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support dispatch_cost_qualification -- --ignored --nocapture
```

Native acceptance executable:

```powershell
rtk bypass cargo build --locked --manifest-path rust/Cargo.toml -p sky_player --release --features real-input-acceptance --bin rt-native-acceptance
```

The representative matrix is reproduced by this PowerShell loop from a clean checkout:

```powershell
$cases = @(
  @{ Scenario = 'dense-alternating'; Reps = 20 },
  @{ Scenario = 'long-single-sequence'; Reps = 20 },
  @{ Scenario = 'mixed-up-down'; Reps = 100 },
  @{ Scenario = 'chord-sweep'; Reps = 20 },
  @{ Scenario = 'near-minimum-retrigger'; Reps = 64 }
)
foreach ($case in $cases) {
  for ($i = 1; $i -le $case.Reps; $i++) {
    & pwsh.exe -NoProfile -File scripts/run_native_acceptance_case.ps1 `
      -Scenario $case.Scenario -TimingMarginUs 500 -TimingObserverForQualification
    if ($LASTEXITCODE -ne 0) { throw "native qualification failed: $($case.Scenario) run $i" }
  }
}
```

The measurement used the supplied repetition counts in the table, `Baseline` calibration placement,
production profile, the observer enabled for qualification, no CPU contention, 500 µs margin, and
the host-default power plan. The runner records the source revision, source-tree cleanliness,
Windows build, processor, Rust version, harness SHA-256, QPC frequency, and per-session evidence.

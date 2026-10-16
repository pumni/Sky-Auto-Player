# R3 keyboard final qualification evidence (P5)

P5 is a final characterization checkpoint for [issue #436](https://github.com/pumni/Sky-Auto-Player/issues/436), based on the merged P4 runtime. The evidence collection completed, but this checkpoint is **not qualified for merge**: the broad dispatch assembly audit reports a retained recovery-path copy and requires coordinator review. The focused prepared-normal and modifier/lease audits pass. No production behavior was changed to hide or work around the broad audit finding.

P5 adds final acceptance scenarios and test-support seams, the final evidence runner, and its published evidence bundle. It does not change production scheduling, timing, timer, priority, SendInput, or input policy. P0–P4 reports and evidence remain immutable.

## Phase and revision crosswalk

The accepted PRs below were merged with normal merge commits. Each PR head and reviewed base is retained so the runtime lineage is reproducible.

| Phase | Issue | PR | Reviewed base | Reviewed head | Merge commit |
| --- | --- | --- | --- | --- | --- |
| P0 characterization | [#431](https://github.com/pumni/Sky-Auto-Player/issues/431) | [#437](https://github.com/pumni/Sky-Auto-Player/pull/437) | `4bd103c21a3fd8aad15eff281db15f8b10409493` | `1b6afb671d10a89cd632564ae772de6094850bb9` | `aa3c1a97c6a66c4036fed2c27617d0b3069773fd` |
| P1 lifecycle release barrier | [#432](https://github.com/pumni/Sky-Auto-Player/issues/432) | [#447](https://github.com/pumni/Sky-Auto-Player/pull/447) | `aa3c1a97c6a66c4036fed2c27617d0b3069773fd` | `b28f75126d091b86b40bb41926526a78bed0b73e` | `b3324655c1b4be82849fc9b2d858541505d69963` |
| P2 final lease age | [#433](https://github.com/pumni/Sky-Auto-Player/issues/433) | [#448](https://github.com/pumni/Sky-Auto-Player/pull/448) | `b3324655c1b4be82849fc9b2d858541505d69963` | `c9bea5e6593f576cc3c1b72d91633d628c37e900` | `df99c4080c556de163ea9aea7465ca70156b5739` |
| P3 contract documentation | [#434](https://github.com/pumni/Sky-Auto-Player/issues/434) | [#449](https://github.com/pumni/Sky-Auto-Player/pull/449) | `df99c4080c556de163ea9aea7465ca70156b5739` | `e511f2223f32df5a08160a73dc8a714d372641ac` | `b012b0a1c135a9176cfc7c6aa8696fa0be29a481` |
| P4 diagnostics | [#435](https://github.com/pumni/Sky-Auto-Player/issues/435) | [#450](https://github.com/pumni/Sky-Auto-Player/pull/450) | `b012b0a1c135a9176cfc7c6aa8696fa0be29a481` | `cb40f6b3e77162cf66f1ef26408d6b7546ffd034` | `bf5af844aa9e98f94b6cea8b81b3edb9a1d066ed` |
| P5 final evidence | [#436](https://github.com/pumni/Sky-Auto-Player/issues/436) | Draft PR for this checkpoint | `bf5af844aa9e98f94b6cea8b81b3edb9a1d066ed` | Runtime evidence source: `be795506cc7b083673c30eca11d4627695dbad5a` | Pending coordinator review |

The measured runtime revision is `bf5af844aa9e98f94b6cea8b81b3edb9a1d066ed`. The P5 evidence runner and acceptance fixtures were measured from clean source revision `be795506cc7b083673c30eca11d4627695dbad5a`; that revision is not a new production runtime revision. The paired control is the accepted P1 merge `b3324655c1b4be82849fc9b2d858541505d69963`. The documentation-only PR head and its exact-head CI are recorded in the #436 checkpoint; the runtime evidence remains bound to `be795506cc7b083673c30eca11d4627695dbad5a`.

## Host and executable provenance

The final run used Windows 11 Pro build 26300 x64 MSVC, Rust/Cargo 1.99.1, Bun 1.4.3, PowerShell 7.6.6, an AMD Ryzen 5 5600, and the Balanced power plan.

The exact collection command was:

```powershell
pwsh -NoProfile -File scripts/run_rt_r3_evidence.ps1 -Stage final -OutputRoot .benchmarks/r3/p5 -ControlRevision b3324655c1b4be82849fc9b2d91633d628c37e900
```

It produced run `r3-p5-20261016T171202803Z-4e4fc2d1`, 259 child-command records, and status `COMPLETED_CHARACTERIZATION`. Its separate qualification fields are `COORDINATOR_REVIEW_REQUIRED` and `qualification_passed=false` because of the broad assembly finding.

| Artifact | SHA-256 |
| --- | --- |
| P1-control precision executable | `643523413e6539492289e7eaeeb11e68f2323729a5f56747b5b1bcb2207a2733` |
| Final candidate precision executable | `f1019540b9757d40577416636c946db09849fbc7cd9624506038d0b9a4a1a6d5` |
| Native acceptance harness executable | `025403060795198143d3c0031f45d04c19f19feb7580dded36bcb50953ff7d38` |
| Native acceptance runner script | `686939799f9e8866b12d70d4d4ade7950998b90e9fc006bc27f4a51a2256cf5c` |
| Optimized `sky_player` assembly | `07863e2ef8037707ad52fde4d4ab3b0aed4a2d70b43800e47f996487c06ba6aa` |
| Optimized `sky_dispatch_win32` assembly | `3e61e688d96ced1c6fe888bcb5c5f9369a858dc4b1a2d6ce0d7f83a1e3f6fefe` |

## Verification results

The final evidence runner invoked these gates from source revision `be795506cc7b083673c30eca11d4627695dbad5a`. Every command exited 0 except the broad dispatch assembly audit, which is recorded separately below.

| Gate | Result |
| --- | --- |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib r3_` | 17 passed, including `r3_late_up_empty_pause_resume_preserves_release_floor` |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core` | 73 passed across test targets |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 248 passed, 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib` | 385 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 25 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features real-input-acceptance,test-support --bin rt-native-acceptance` | 22 passed |
| `cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --no-default-features --features tauri-test native_runtime` | 79 passed |
| `cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --no-default-features --features tauri-test power_lifecycle` | 9 passed |
| `cargo xtask check static` | Passed |
| `cargo xtask check all` | Passed |
| `git diff --check` | Passed |
| `cargo build --locked --manifest-path rust/Cargo.toml -p sky_player --release --features real-input-acceptance,test-support --bin rt-native-acceptance` | Passed |
| `pwsh -NoProfile -File scripts/native_acceptance_sink.ps1 -SelfTest` | Passed |

The native case fixture now expects an empty sink window for delayed lease expiry while retaining an authored Down packet after the lease timeout. The native oracle must show the Down was rejected before the sender. Its unit test asserts both the delayed Down schedule and the zero-event expectation. No production code was changed for this correction.

## Native acceptance matrix

The final runner executed all 30 native acceptance scenarios at 500 µs, six specified scenarios at each of 0, 100, and 3,000 µs, and three contention repetitions each for `dense-alternating` and `canonical-max-chord` at 500 µs. This is 54 runs: 36 at 500 µs, six each at 0, 100, and 3,000 µs, and six contention runs. **All 54 passed**, with 4,826 ReceiveOnly sink events reconciled. No case was marked `CASE-NOT-EXERCISED`.

The matrix used the project-owned ReceiveOnly sink and retained its run ID, HWND, process start time, event-log identity, cursor, event window, raw events, and per-run executable and runner hashes. Abnormal scenarios used their dedicated terminal and event-window oracles.

The controlled pause/resume release-floor scenario completed 20 of 20 cycles. Every barrier used the fresh post-cleanup QPC sample plus a 166,670-tick frame (16,667 µs at 60 FPS). No resumed Down started before its barrier, all 20 floor checks passed, and the case delivered 42 expected sink events. Across cycles, the authored target was 12,128 to 74,584 QPC ticks before the barrier; the actual sender began 54 to 83 ticks after the barrier. There were zero production release-floor violations.

The delayed-watchdog lease-age scenario observed the watchdog held before the age check and waiting at the controlled barrier. After session supervisor progress stopped, the delayed authored Down was rejected by the final worker proof: `down_sender_attempt_count=0`, `final_gate_lease_expirations=1`, terminal error `supervisor_lease_expired`, and 174 independent helper liveness ticks. The expected sink event count and observed sink event count were both zero. The existing supervisor lease expiry control also passed.

The focus-loss scenario passed with four sink events and the expected zero-event probe window. The P4 polling model remains explicit that a receiver may miss a short hold; the native sink proves only delivery to that sink.

## Paired performance qualification

The final runtime was compared with the accepted P1 control in a fixed, paired A/B/BA sequence: `AB, BA, AB, BA, AB`. The matrix covered nine workloads (`down-1/5/15`, `mixed-1x1/2x3/7x8`, and `up-only-1/5/15`) under both quiet and CPU-contention load. Each of the 18 workload/load cells had five A/B pairs, 1,000 warm-up samples and 10,000 measured samples per run: 90 pairs, 180 runs, and 1,800,000 measured samples total.

All 18 comparisons passed their configured p99 budgets. At the probe's integer-microsecond resolution, the median run p99 was 0 µs for both control and candidate in every cell; the maximum candidate-minus-control p99 delta was 0 µs. Raw per-run distributions are retained in the evidence archive. These deterministic precision measurements are not Windows SendInput latency measurements; real SendInput delivery was measured separately against the ReceiveOnly sink.

## Optimized assembly audits

The optimized `sky_player` and Win32 assembly used the `dist` profile. The scoped prepared-normal audit passed with one prepared sender call, one prepared-boundary QPC sample, one post-modifier/pre-control lease-age QPC sample, zero retained precision helper calls, zero scoped forbidden calls, and zero scoped copy/division symbols. The modifier/lease audit passed with the five configured modifier probes, one prepared dispatch modifier probe, one prepared sender transaction, one authored-admission lease-age QPC sample, one final-policy QPC sample, and zero allocation, mapping, foreground/process query, QPC, lock, or log calls from the fixed modifier probe path.

The broad dispatch audit exited 1 and is **REVIEW_REQUIRED**. It reports one 544-byte `memcpy` in `dispatch_due_from_plan`, on the existing exceptional mixed-boundary recovery path, copying a `PreparedAuthoredCommit` into `PendingUpRecovery`. The raw assembly and audit output are retained. ThinLTO omitted a standalone `recover_missed_down_boundary` symbol; the audit reports `INLINED_OR_NOT_FOUND` and includes its shipping caller, so the absent standalone symbol is not treated as proof. P5 did not change production behavior to remove the copy. The final status is therefore `COMPLETED_CHARACTERIZATION`, `COORDINATOR_REVIEW_REQUIRED`, `qualification_passed=false`.

## Append-only evidence and prior provenance

P5 run `r3-p5-20261016T171202803Z-4e4fc2d1` was published append-only from source `be795506cc7b083673c30eca11d4627695dbad5a`:

- [Evidence run](https://github.com/pumni/Sky-Auto-Player/tree/0687f46dc6924777e75511b5fe73cab556ce61fb/runs/r3-p5-20261016T171202803Z-4e4fc2d1), commit `0687f46dc6924777e75511b5fe73cab556ce61fb`.
- [Manifest](https://github.com/pumni/Sky-Auto-Player/blob/0687f46dc6924777e75511b5fe73cab556ce61fb/runs/r3-p5-20261016T171202803Z-4e4fc2d1/manifest.json), SHA-256 `3f338b2f1ae444dbc2cd2f8e864b0e4801539469d867d92c3ec14c530a968e61`.
- Archive: `archive/evidence.zip`, SHA-256 `418b0d8d60a5d1b64d8c703c6938393d3c820529a1107ceff43f77071c586b5e`.

P5 fetched and verified the P4 append-only artifact before running. P4 provenance is unchanged: evidence commit [`5c0445a7d5965fd2320856df60e260372586f341`](https://github.com/pumni/Sky-Auto-Player/tree/5c0445a7d5965fd2320856df60e260372586f341/runs/r3-p4-20261016T145957238Z-be3ec0ab), run `r3-p4-20261016T145957238Z-be3ec0ab`, manifest SHA-256 `b11bbb81732edc6ec6b7c61fe878e7df6621c99c25efcefa2a56435eac11d028`. All 27 declared P4 raw outputs were verified. The P4 characterization is documented in [r3-keyboard-diagnostics.md](r3-keyboard-diagnostics.md).

P0, P1, and P2 evidence remains linked from [ADR-0018's published phase evidence](adr/ADR-0018-resumable-release-barrier-and-final-lease-age.md#published-phase-evidence). The merged P3 contract changes and P4 diagnostics are covered by their linked PRs and current docs. No P0–P4 evidence file was changed or replaced by P5.

## Contract boundary and remaining review

The evidence supports sender-side lifecycle and admission checks: resumable Downs are held behind the actual post-cleanup release floor, and an expired final worker lease rejects an eligible Down before the sender. The broad assembly finding remains for coordinator review, so P5 is not marked passed or merge-ready here.

ReceiveOnly records prove that Windows delivered the expected events to the project-owned sink. They do not prove that Sky polled the key state, accepted a note, rendered an onset, or produced audio. Receiver observation remains unknown; short holds may be missed.

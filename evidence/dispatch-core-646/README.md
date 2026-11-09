# Evidence — Issue #646, package A (review resubmission)

This directory contains the readable report and path-redacted raw evidence for the Windows runs below. The archived JSONL, logs, runner inputs, ReceiveOnly sink events, and invocation summaries preserve each run's reported outcome. Redaction replaces only local absolute checkout/temp roots; it does not change scenario results. `ORIGINAL-SHA256SUMS.txt` records hashes of the original local ZIPs before redaction. `SHA256SUMS.txt` verifies the files published here.

## Source and executable identity

The C0 executable remains frozen from the clean A1 measurement-foundation commit; it was not rebuilt or changed during A2.

| | Frozen control C0 | Candidate at last native run |
| --- | --- | --- |
| Source commit / tree | `b52106b47e4163d0906abbaa59d343935f4fba42` / `0e1933cc565ad8f9b42239312c996facf2a5126a` | `27bea42d063ebad86cc04e45a71cd18bd14fd482` / `47cbb8cd07a22e47b2fa6059943b95c304976087` |
| Branch / clean source | `perf/dispatch-core-20261109`, clean | `perf/dispatch-core-20261109`, clean |
| Profile / target | `dist`, `x86_64-pc-windows-msvc` | `dist`, `x86_64-pc-windows-msvc` |
| Features / QPC counter | `real-input-acceptance`, off | `real-input-acceptance`, off |
| Binary SHA-256 | `98017d8e20d938681ba43cc57f62cdb561248583fc52880628e3d36c12d6b569` | `4c83c5ce433346484b779a94b645e64fa95af3ad482ca644b3871128a4bb8b9b` |
| Original manifest SHA-256 | `b9e51334b9169e039db945028bedf725909fb595342d277b41291cda06a1e544` | `386dd3f48797ce0f18439ddd4c58d7d1141516d91d1a3e570200e47ed6c95f85` |
| Published path-redacted manifest SHA-256 | `af149a8b88708ee5a6db1fd2ca383a6b27508fa1a81aa402a1d66bed35d4733f` | `9e6261a343cbb3e299e0a888210516ea8005f85a4e500a2b4aab1539b87c5e0d` |

The immutable manifest files are [`C0`](./a1-c0-b52106b4.manifest.json) and [`candidate`](./a2-candidate-27bea42.manifest.json). Candidate smoke was built after the source commit above; the later evidence commit only adds this report and bundles. The binaries themselves remain outside the checkout as required by the runbook.

## Safety evidence

All native harness traffic used the project's identity-verified **ReceiveOnly** sink. No input was sent to the game. The verdicts below are the recorded verdicts; later focused reproductions do not overwrite full-Smoke results.

| Run | PASS | INCONCLUSIVE | FAIL | Process result |
| --- | ---: | ---: | ---: | --- |
| Frozen C0 Smoke, 22 cases | 18 | 3 | 1 | exit 1 |
| Candidate Smoke at `27bea42`, 22 cases | 20 | 1 | 1 | exit 1 |
| Candidate focused `pause-resume-release-floor`, 20 cycles | 1 focused case | 0 | 0 | exit 0 |

### C0 failures and inconclusives — individually reproduced

The direct reproductions used the unchanged C0 binary and manifest. Their original verdicts remain recorded in the C0 Smoke archive and the targeted reproduction bundle.

| Scenario | C0 recorded result | Raw evidence and diagnosis |
| --- | --- | --- |
| `lease-age-watchdog-delayed` | **FAIL** | The report classifier says “transport or residue anomaly.” The native report instead records terminal `supervisor_lease_expired`, final-gate lease expirations `1`, Down sender attempts `0`, helper liveness `175`, empty ReceiveOnly sink (expected `0`, contiguous sequence), `active=0`, `possibly_active=0`, no release obligation, cleanup verified, and no transport/residue anomaly. This points to an assertion/classifier mismatch around an expected terminal block, not an observed input/sink safety violation. The original FAIL is **not** relabeled; C0 remains unqualified until the leader adjudicates the mismatch. |
| `preflight-user-held` | **INCONCLUSIVE** | The direct raw report shows startup preflight rejected held scan code `21`, with `0` inserted keys, no active/possibly-active key, no release obligation, no cleanup call and no sender attempt. The C0 report cannot take timing telemetry because it had already been consumed; the direct report also marks the sink report-window match false. The safety facts are retained, but the C0 verdict stays INCONCLUSIVE. |
| `owner-process-termination` | **INCONCLUSIVE** | The reproduction invalidates synthetic owner authority during preparation; it does **not** terminate the OS owner process. No key was sent and there was no release obligation. The missing telemetry (“already been taken”) and non-OS fixture mean this is not evidence for actual process-termination behavior. |
| `suspend-resume` | **INCONCLUSIVE** | The old C0 fixture notified system suspend, waited for a pause to commit, then notified resume and expected the full authored sequence. Under Issue #646, system suspend is terminal for the physical session, so this expectation conflicts with the authorized contract. The C0 fixture result remains INCONCLUSIVE. The candidate fixture now asserts terminal suspend without a resume notification and checks Down plus safety Up; that candidate case passes, but it does not change C0's verdict. |

The four C0 raw reproductions are in [`c0-targeted-p0-fbe703ab.redacted.zip`](./c0-targeted-p0-fbe703ab.redacted.zip); the immutable-C0 22-case archive is [`c0-smoke-b52106b4.redacted.zip`](./c0-smoke-b52106b4.redacted.zip). The earlier submitted candidate run is retained in [`a2-original-review-fbe703ab.redacted.zip`](./a2-original-review-fbe703ab.redacted.zip).

### Candidate Smoke findings

Candidate Smoke records 20 PASS, one owner-process-termination INCONCLUSIVE and one `pause-resume-release-floor` FAIL. The FAIL reason is `CASE-NOT-EXERCISED`: at cycle 1 the resumed authored Down target was later than the release floor, so the fixture precondition did not occur. This is not a demonstrated floor violation, but the Smoke verdict remains FAIL. A same-protocol focused rerun on the immutable candidate exercised the intended precondition and passed 20 cycles (42 ReceiveOnly sink events); it is supplementary evidence only. Raw full and focused reports are [`a2-smoke-27bea42.redacted.zip`](./a2-smoke-27bea42.redacted.zip) and [`a2-focused-qpc-release-floor-27bea42.redacted.zip`](./a2-focused-qpc-release-floor-27bea42.redacted.zip).

The owner-process-termination candidate fixture likewise only invalidated synthetic owner authority before the sender and explicitly reports `os_process_terminated: false`; it remains INCONCLUSIVE. Candidate suspend is terminal, with no simulated resume, and its cleanup left no outstanding obligation. The older `d417b0c` suspend Smoke is retained because its assertion still expected the full authored event sequence after terminal suspend; raw evidence is [`a2-suspend-fixture-d417b0c.redacted.zip`](./a2-suspend-fixture-d417b0c.redacted.zip).

An earlier `c0977bf` candidate Smoke attempt stopped in the runner with a PowerShell missing-property error (`counts`); the runner output bug was corrected and the original failure retained in [`a2-runner-failure-c0977bf.redacted.zip`](./a2-runner-failure-c0977bf.redacted.zip).

## Measurement and benchmark status

The runner now reports separate per-chord authored lateness, floor displacement and residual lateness; waiter-to-pre-call and per-syscall sender envelope; per-key H/G; Down/Up/Mixed packet-kind and chord-width/hot-cold groups; sample counts and missing/negative-order/drop provenance; CPU/spin availability; and per-cell paired p95/p99 regression screens. Unavailable metrics remain unavailable, not zero. `Get-PairedMetrics` is implemented but **Compare was not run**: the frozen C0 safety Smoke still has one FAIL and three INCONCLUSIVE, so the accepted baseline gate is not met. There are no paired performance results or performance claims in this report.

| Diagnostic evidence | Result |
| --- | --- |
| Candidate timed binary | `qpc-read-counting` disabled; SHA above |
| Counting-only binary | `real-input-acceptance,qpc-read-counting`; SHA `8f50f423cdc6794775d91f0cb99b4be32f134d8e2a1af7adeb24fc201f2bdf35`; manifest SHA before path redaction `0efe0f56857a2615c20343f4c21da2946e0e709a022a7906234f627745548e2e`, published manifest SHA `6445f5bd72b13be3d332e88ad42c49d83de6f953ba8e0c2abfd1d3ebd04122b6` |
| Counting-only native case | `canonical-single` PASS; `25,093` QPC reads, 2 SendInput attempts, 2 accepted telemetry records, 2 sink events; exit 0 |
| Default optimized assembly | SHA-256 `8ac5fa8df3793a2c5d2dd210ce099c4bbf4cd5d754fc9fba802840c8cad2d3a7`; `QpcClock::now` calls `QueryPerformanceCounter`, no `QPC_READ_COUNT` atomic increment |
| Counting optimized assembly | SHA-256 `8c639d8037946665da477018ebee62cca7bd042cc9c9f4e010da816fd21fc5ad`; `QpcClock::now` adds `lock incq QPC_READ_COUNT` before the same API call |
| Snippet file | [`qpc-assembly-snippets.txt`](./qpc-assembly-snippets.txt), SHA-256 `7ab4450652c39c57039684a9fa2820629300d2e78a3a0ab21c2926619858880b4` |

The complete counting run, both manifests, assembly excerpt, focused release-floor run, and ReceiveOnly sink evidence are bundled in [`a2-focused-qpc-release-floor-27bea42.redacted.zip`](./a2-focused-qpc-release-floor-27bea42.redacted.zip). Counting/assembly evidence is diagnostic and is not from the timed candidate binary.

## Commands and exit codes

Commands were run on Windows 11 using the repository-pinned Rust 1.99.1 toolchain and target `x86_64-pc-windows-msvc`.

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all` | 0 | pass |
| `cargo test --manifest-path rust/Cargo.toml -p sky_player --bin rt-native-acceptance --features real-input-acceptance --locked` | 0 | 30 tests passed |
| `rtk cargo xtask check all` (repository root) | 0 | full local gate passed: architecture/static/security, vet, fmt, clippy all-features, workspace tests, Bun checks/E2E and Tauri checks |
| `pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 -OutputRoot <candidate-output>` | 0 | clean candidate build and manifest |
| `pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 -Stage Smoke -CandidateManifest <candidate-manifest> -TimingMarginUs 500 -OutputRoot <smoke-output>` | 1 | 20 PASS, 1 INCONCLUSIVE, 1 FAIL; preserved above |
| `pwsh -NoProfile -File scripts/run_native_acceptance_case.ps1 -Scenario pause-resume-release-floor -TimingMarginUs 500 -CalibrationPlacement Baseline -TimingObserverForQualification -HarnessPath <immutable-candidate> -HarnessManifestPath <candidate-manifest> -OutputRoot <focused-output> -GameFps 60` | 0 | 20 intended cycles passed |
| `pwsh -NoProfile -File scripts/run_native_acceptance_case.ps1 -Scenario canonical-single -TimingMarginUs 500 -CalibrationPlacement Baseline -TimingObserverForQualification -QpcReadCountingEvidenceOnly -HarnessPath <counted-binary> -HarnessManifestPath <counted-manifest> -OutputRoot <counted-output> -GameFps 60` | 0 | counting-only evidence above |
| Default and `qpc-read-counting` `cargo rustc --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --lib --profile dist --target x86_64-pc-windows-msvc -- --emit=asm` builds (counted build adds `--features qpc-read-counting`) | 0 each | assembly emitted; compiler warned about an unrelated unused `ActiveGeneration` import in `authored.rs`; full clippy gate passed |
| `git diff --check` | 0 | pass before evidence commit |

An earlier `cargo xtask check all` invocation from `rust/` exited 1 because the repository alias resolves `rust/xtask/Cargo.toml` relative to the current directory. The documented full gate was rerun from repository root and passed. A runner-only Smoke attempt at `c0977bf` also failed before scenarios due missing `counts`; the failure artifact is retained above and a subsequent full Smoke completed with the results shown.

## Documentation and source-consumer inventory for later D planning

This is the A-related inventory requested by the runbook. It records consumers and drift for later review; **no broad document cleanup was performed in A**.

| Path | Current consumer / role | Duplicate or stale claim to review in D |
| --- | --- | --- |
| `AGENTS.md`, `docs/INDEX.md` | Repository agent entry and locator for architecture, dispatch, timing, and evidence contracts | Index calls timing evidence current; schema/field claims below have advanced. |
| `docs/architecture.md`, `docs/rt-dispatch-architecture.md` | Current architecture/playback contract; linked from agent context, index, and English/Vietnamese website guides | Dispatch architecture still describes telemetry schema 19 and says absolute QPC target fields are absent; candidate exports schema 22 and timer-arm/target/return provenance. Historical qualification examples remain revision-specific. |
| `docs/timing-principles.md`, `docs/rt-dispatch-timing-evidence.md` | Timing contract and trace field definitions consumed by the dispatch docs and measurement workflow | Both describe schema 19. Timing evidence's absolute-QPC field inventory omits the new timer-arm generation/provenance fields. Keep authored/floor absolute target availability distinct from the derived relative timer-arm target. |
| `docs/physical-key-study-method.md`, `scripts/qualify_physical_key_study.ps1` | Methodology and its physical-key qualification runner; linked by dispatch documentation and index | Historical results are revision-bound; do not treat as current A qualification. |
| `docs/prepare-whole-stream-timing-advisory.md`, `apps/desktop/src-tauri/src/native_runtime/playback/timing_advisory.rs`, `apps/desktop/src/components/player/TimingAdvisory.tsx`, `apps/desktop/src-tauri/src/native_runtime/playback/commands.rs` | Read-only Prepare UI whole-stream advisory computation, DTO and display | Actual live consumer; document is not dispatch authorization. Retain/migrate only by later leader-approved manifest. |
| `apps/website/src/content/guides/en/en-timing-engine.md`, `vi/vi-timing-engine.md`, `en/en-how-it-works.md`, `vi/vi-how-it-works.md`, `en/en-troubleshooting.md`, `vi/vi-troubleshooting.md` | Published guides link to timing principles and dispatch architecture | These links are active public consumers of documentation paths; update together if D moves/removes docs. |
| `rust/crates/sky_dispatch_core/src/coordinator/{mod.rs,ownership.rs}`, `rust/crates/sky_player/src/engine/worker/dispatch/observer_wake.rs`, `rust/crates/sky_dispatch_win32/src/wait/hybrid.rs`, `rust/crates/sky_player/src/engine/worker/dispatch/observation.rs` | Prepared commit proof, final admission/wait, timer-arm provenance, telemetry snapshot/export | Runtime source is the authority for current proof revision and schema 22; old docs and historical ADR snapshots may duplicate prior behavior. |
| `scripts/qualify_dispatch_core.ps1`, `scripts/run_native_acceptance_case.ps1` | Smoke/Compare protocol, metric derivation, immutable harness validation and ReceiveOnly native cases | Runner output is current implementation; old benchmark/evidence reports are not current qualification. |
| `docs/adr/ADR-0001*`, `ADR-0010*`, `ADR-0014*`, `ADR-0016*`, `ADR-0017*`, `ADR-0018*` | Historical decisions linked from `docs/INDEX.md`; no current product source consumer found | Revision-bound records. D should decide retention/provenance under the leader's later KEEP/REWRITE/DELETE manifest, not during A. |

## Limitations and gate status

- C0 remains 18/22 with one original FAIL and three original INCONCLUSIVE. The exact C0 executable/manifest are preserved; no baseline substitution or verdict rewrite was made.
- Candidate Smoke remains 20/22 with one FAIL and one INCONCLUSIVE. The focused case is supplementary; leader adjudication is still required.
- The fixed Compare matrix (3 workloads × Quiet/Contention × 6 pairs × 15 seconds, alternating AB/BA) was **not run** because C0 is not safety-qualified. Benchmark fields and paired regression screen are implemented, but there is no valid C0/A performance result.
- The owner-process-termination native fixture does not terminate the OS process; an actual process-termination verdict is not available from these runs.
- Full local checks passed; GitHub CI status is reported separately on PR #647 and may still be pending when this evidence commit is pushed.
- B/C/D remain blocked pending leader acceptance/authorization. No merge, policy/gate change, or unrelated security/updater/release-trust change is included.

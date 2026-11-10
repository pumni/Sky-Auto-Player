# Evidence — Issue #646, package A (leader review #2 follow-up)

This directory contains the readable report, manifests, and checksums for the Windows runs below. The path-redacted raw JSONL, logs, runner inputs, ReceiveOnly sink events, invocation summaries, and original verdicts are in the [GitHub Actions evidence artifact](https://github.com/pumni/Sky-Auto-Player/actions/runs/42617626785), uploaded from commit `ac4d42832d1ce1d1319fdf290aab52ee4ce0311a`. Artifact ID `13672948052` expires `2027-02-07T12:30:45Z`. [`ACTIONS-ARTIFACT-SHA256SUMS.txt`](./ACTIONS-ARTIFACT-SHA256SUMS.txt) lists each nested ZIP hash and the artifact digest; `ORIGINAL-SHA256SUMS.txt` preserves original local ZIP hashes.

## Source and executable identity

The C0 executable remains frozen from the clean A1 measurement-foundation commit; it was not rebuilt or changed during A2.

| | Frozen control C0 | Latest candidate Smoke |
| --- | --- | --- |
| Source commit / tree | `b52106b47e4163d0906abbaa59d343935f4fba42` / `0e1933cc565ad8f9b42239312c996facf2a5126a` | `4b1cec7a874ace7030e1b5e47aeccd315f3d9fe4` / `bf2a1d9c34a55d4885fbde53a661d152b0517bb7` |
| Branch / clean source | `perf/dispatch-core-20261109`, clean | `perf/dispatch-core-20261109`, clean |
| Profile / target | `dist`, `x86_64-pc-windows-msvc` | `dist`, `x86_64-pc-windows-msvc` |
| Features / QPC counter | `real-input-acceptance`, off | `real-input-acceptance`, off |
| Binary SHA-256 | `98017d8e20d938681ba43cc57f62cdb561248583fc52880628e3d36c12d6b569` | `5cddfc1f32bd3e0b8d3ccb7ea2803f6c34aba929847c6abfcf0c86af7b28e7ae` |
| Original manifest SHA-256 | `b9e51334b9169e039db945028bedf725909fb595342d277b41291cda06a1e544` | `b81794a5d33958213e4778ac1087fa409e52b24f811ac8541a9c1b69f2fa1b49` |
| Published path-redacted manifest SHA-256 | `af149a8b88708ee5a6db1fd2ca383a6b27508fa1a81aa402a1d66bed35d4733f` | `955022bcd21ab1ceadcb2fa8192705b144a682fdc89c290ab15bde3dcb46434d` |

The immutable manifest files are [`C0`](./a1-c0-b52106b4.manifest.json) and [the latest candidate](./a2-candidate-4b1cec7.manifest.json). The candidate was built from clean source at the recorded commit; the binaries remain outside the checkout.

## Safety evidence

All native harness traffic used the project's identity-verified **ReceiveOnly** sink. No input was sent to the game. The verdicts below are the recorded verdicts; later focused reproductions do not overwrite full-Smoke results.

| Run | PASS | INCONCLUSIVE | FAIL | Process result |
| --- | ---: | ---: | ---: | --- |
| Frozen C0 Smoke, 22 cases | 18 | 3 | 1 | exit 1 |
| Candidate Smoke at `4b1cec7`, 22 cases | 22 | 0 | 0 | exit 0 |
| Prior candidate Smoke at `5e0b5b3`, 22 cases | 21 | 0 | 1 | exit 1 |

### C0 failures and inconclusives — individually reproduced

The direct reproductions used the unchanged C0 binary and manifest. The [external requalification report](../../docs/HISTORY.md#issue-646-qualification-evidence-phase-a2--c0) yields 21 PASS and 1 BLOCKED; it retains each original native verdict and does not upgrade the original `FAIL` or `INCONCLUSIVE` records. The full JSON/CSV/script bundle `c0-requalification-b52106b4-r4.redacted.zip` is in the Actions artifact.

| Scenario | C0 recorded result | Raw evidence and diagnosis |
| --- | --- | --- |
| `lease-age-watchdog-delayed` | **FAIL** | The original report classifier says “transport or residue anomaly,” while the retained native report records terminal `supervisor_lease_expired`, one final-gate lease expiry, zero Down sender attempts, a live helper, an empty identity-bound ReceiveOnly window, zero active/possibly-active keys or obligations, and clean verified release. The external oracle qualifies the terminal fail-closed behavior but retains the original FAIL verbatim. |
| `preflight-user-held` | **INCONCLUSIVE** | Raw evidence shows startup preflight rejected scan code `21`, with zero inserted keys, sender attempts, cleanup transport, active/possibly-active keys or release obligations, and an empty identity-bound ReceiveOnly window. The original telemetry-unavailable INCONCLUSIVE remains recorded; the external safety oracle passes the preflight behavior. |
| `owner-process-termination` | **INCONCLUSIVE / BLOCKED** | C0's immutable fixture only invalidated synthetic owner authority during preparation (`owner_authority_invalidated_during_preparation`); its raw report has no OS termination fixture. This does **not** prove termination of the owner process. The external result is BLOCKED, with no verdict rewrite. |
| `suspend-resume` | **INCONCLUSIVE** | C0 reports terminal `system_suspend`, the expected key Down followed by safety Up on the ReceiveOnly sink, verified cleanup and zero residual obligations. The old fixture's resume/full-song expectation is outside the authorized terminal-suspend semantics. The external oracle passes the terminal prefix while preserving the original INCONCLUSIVE. |

The four C0 raw reproductions (`c0-targeted-p0-fbe703ab.redacted.zip`), immutable-C0 22-case archive (`c0-smoke-b52106b4.redacted.zip`), latest external oracle (`c0-requalification-b52106b4-r4.redacted.zip`), and earlier submitted candidate run (`a2-original-review-fbe703ab.redacted.zip`) are in the Actions artifact.

### Candidate Smoke findings

The latest candidate Smoke at `4b1cec7` records **22/22 PASS**. Its [readable summary](../../docs/HISTORY.md#issue-646-qualification-evidence-phase-a2--c0) and raw bundle `a2-smoke-4b1cec7.redacted.zip` preserve all case evidence in the Actions artifact. The owner-process test independently verified `TerminateProcess` on the identity-bound ReceiveOnly sink (exit code `0x646`), zero sink events and sender attempts, no release obligations, and fail-closed terminal `focus_lost_during_preroll`. The release-floor case completed 20 cycles; each resumed authored target was observed due before its Q+T barrier, no Down sender attempt occurred before the barrier, and all 20 Down/Up pairs completed. First-cycle QPCs: target `2908233817555`, due checkpoint `2908233863456`, barrier `2908233926163`, sender start `2908233926837`.

The earlier `5e0b5b3` candidate Smoke records 21 PASS and one `pause-resume-release-floor` FAIL. Its failure was `CASE-NOT-EXERCISED`: the prior checkpoint sampled before the resumed target could be observed; the native output did not report a floor violation. That recorded failure remains preserved in `a2-smoke-5e0b5b3.redacted.zip` in the Actions artifact. The corrected runner captures the prepared target/window before its floor wait, and the later `4b1cec7` full Smoke passes. The previous `27bea42` full Smoke, focused run and `d417b0c` suspend-fixture run are also retained in the artifact with their original results.

An earlier `c0977bf` candidate Smoke attempt stopped in the runner with a PowerShell missing-property error (`counts`); the runner output bug was corrected and the original failure remains in `a2-runner-failure-c0977bf.redacted.zip` in the Actions artifact.

## Measurement and benchmark status

The runner reports separate per-chord authored lateness, floor displacement and residual lateness; waiter-to-pre-call and per-syscall sender envelope; per-key H/G; Down/Up/Mixed packet-kind and chord-width/hot-cold groups; sample counts and missing/negative-order/drop provenance; CPU/spin availability; and separate per-cell p95/p99 paired regression screens. Unavailable metrics remain unavailable, not zero. `Get-PairedMetrics` is implemented but **Compare was not run**: C0 external qualification is still BLOCKED on actual owner process termination. There are no paired performance results or performance claims in this report.

| Diagnostic evidence | Result |
| --- | --- |
| Candidate timed binary | `qpc-read-counting` disabled; SHA above |
| Counting-only binary | `real-input-acceptance,qpc-read-counting`; SHA `8f50f423cdc6794775d91f0cb99b4be32f134d8e2a1af7adeb24fc201f2bdf35`; manifest SHA before path redaction `0efe0f56857a2615c20343f4c21da2946e0e709a022a7906234f627745548e2e`, published manifest SHA `6445f5bd72b13be3d332e88ad42c49d83de6f953ba8e0c2abfd1d3ebd04122b6` |
| Counting-only native case | `canonical-single` PASS; `25,093` QPC reads, 2 SendInput attempts, 2 accepted telemetry records, 2 sink events; exit 0 |
| Default optimized assembly | SHA-256 `8ac5fa8df3793a2c5d2dd210ce099c4bbf4cd5d754fc9fba802840c8cad2d3a7`; `QpcClock::now` calls `QueryPerformanceCounter`, no `QPC_READ_COUNT` atomic increment |
| Counting optimized assembly | SHA-256 `8c639d8037946665da477018ebee62cca7bd042cc9c9f4e010da816fd21fc5ad`; `QpcClock::now` adds `lock incq QPC_READ_COUNT` before the same API call |
| Snippet file | [`qpc-assembly-snippets.txt`](./qpc-assembly-snippets.txt), SHA-256 `7ab4450652c39c57039684a9fa2820629300d2e78a3a0ab21c2926619858880b4` |

The complete counting run, both manifests, assembly excerpt, focused release-floor run, and ReceiveOnly sink evidence are bundled in `a2-focused-qpc-release-floor-27bea42.redacted.zip` in the Actions artifact. Counting/assembly evidence is diagnostic and is not from the timed candidate binary.

## Commands and exit codes

Commands were run on Windows 11 using the repository-pinned Rust 1.99.1 toolchain and target `x86_64-pc-windows-msvc`.

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all` | 0 | pass |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --bin rt-native-acceptance --features real-input-acceptance` | 0 | 32 tests passed |
| `rtk cargo xtask check all` (repository root) | 0 | full local gate passed: architecture/static/security, vet, fmt, Clippy all-features, workspace tests, Bun checks/E2E and Tauri checks. Architecture audit reported 55 allowlisted warnings, not failures. |
| `pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 -OutputRoot <candidate-output>` | 0 | clean candidate build and manifest |
| `pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 -Stage Smoke -CandidateManifest <candidate-manifest> -TimingMarginUs 500 -OutputRoot <smoke-output>` at `4b1cec7` | 0 | 22/22 native Smoke cases PASS, including OS-verified owner termination and 20 release-floor cycles |
| Same full Smoke command at `5e0b5b3` | 1 | Original 21 PASS / 1 FAIL retained; release-floor precondition was not observable before the barrier |
| `pwsh -NoProfile -File scripts/requalify_dispatch_core_c0.ps1 -OriginalSmokeRoot <frozen-C0-smoke> -TargetedReproductionRoot <C0-targeted-runs> -ControlManifestPath <frozen-C0-manifest> -OutputRoot <requalification-output>` | 1 | 21 PASS / 1 BLOCKED / 0 FAIL in external oracle; C0 owner fixture has no OS process-termination proof, so Compare remains blocked |
| `pwsh -NoProfile -File scripts/run_native_acceptance_case.ps1 -Scenario pause-resume-release-floor -TimingMarginUs 500 -CalibrationPlacement Baseline -TimingObserverForQualification -HarnessPath <immutable-candidate> -HarnessManifestPath <candidate-manifest> -OutputRoot <focused-output> -GameFps 60` | 0 | 20 intended cycles passed |
| `pwsh -NoProfile -File scripts/run_native_acceptance_case.ps1 -Scenario canonical-single -TimingMarginUs 500 -CalibrationPlacement Baseline -TimingObserverForQualification -QpcReadCountingEvidenceOnly -HarnessPath <counted-binary> -HarnessManifestPath <counted-manifest> -OutputRoot <counted-output> -GameFps 60` | 0 | counting-only evidence above |
| Default and `qpc-read-counting` `cargo rustc --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --lib --profile dist --target x86_64-pc-windows-msvc -- --emit=asm` builds (counted build adds `--features qpc-read-counting`) | 0 each | assembly emitted; compiler warned about an unrelated unused `ActiveGeneration` import in `authored.rs`; full clippy gate passed |
| `git diff --check` | 0 | pass before evidence commit |

An earlier `cargo xtask check all` invocation from `rust/` exited 1 because the repository alias resolves `rust/xtask/Cargo.toml` relative to the current directory. The documented full gate was rerun from repository root and passed. A runner-only Smoke attempt at `c0977bf` failed before scenarios due missing `counts`; that failure remains archived. A later candidate Smoke failure at `5e0b5b3` is also retained and is not overwritten by the passing run at `4b1cec7`.

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

- Original C0 Smoke remains 18 PASS, 3 INCONCLUSIVE and 1 FAIL. The external oracle reconciles 21 cases but leaves `owner-process-termination` BLOCKED because frozen C0 only performs synthetic authority invalidation; no actual owner OS process termination can be proved from that binary. The exact C0 executable/manifest and all original verdicts remain unchanged.
- Latest candidate Smoke at `4b1cec7` is 22/22 PASS. The prior `5e0b5b3` FAIL remains archived; it was an unexercised runner precondition, not evidence of a physical floor violation.
- The fixed Compare matrix (3 workloads × Quiet/Contention × 6 pairs × 15 seconds, alternating AB/BA) was **not run** because C0 qualification is BLOCKED. Benchmark fields and separate p95/p99 paired regression screens are implemented, but there is no valid C0/A performance result.
- Candidate OS owner-process termination is verified on the identity-bound ReceiveOnly sink. C0 cannot satisfy the same proof with its immutable, older fixture; this is the sole remaining C0 qualification blocker.
- Full local checks passed; at the last lookup the evidence-upload job succeeded and the remaining CI jobs were still in progress on PR #647.
- B/C/D remain blocked pending leader acceptance/authorization. No merge, policy/gate change, or unrelated security/updater/release-trust change is included.

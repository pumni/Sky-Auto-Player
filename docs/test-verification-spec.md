# Test reliability and verification simplification — SPEC v1.0

Status: leader-approved design for staged execution; only a separately dispatched work package authorizes implementation.
Research baseline: [5bab33fa](https://github.com/pumni/Sky-Auto-Player/tree/5bab33faefef97d15e0625d98b1e0eadd209966e), 2026-11-28. Integration branch: `refactor/source-architecture`; existing cumulative [PR #690](https://github.com/pumni/Sky-Auto-Player/pull/690). Previous architecture program: [#689](https://github.com/pumni/Sky-Auto-Player/issues/689). New work does not retroactively change its evidence.
Execution and acceptance: [implementation plan](test-verification-implementation-plan.md). Current owner contracts remain discoverable via [INDEX](INDEX.md).

## 1. Goal and non-goals

Make test oracles causal and independent; reduce false passes, scheduler-sensitive failures, unrelated execution and duplicated verification; improve developer/agent feedback without weakening safety or delivery guarantees. Prefer existing language/tool facilities and executable contracts over a new framework. Do not optimize counts, file length or a green badge in isolation.

Not a product rewrite, native timing optimization, UI redesign, dependency upgrade program, crate redesign, release/security policy change or universal test abstraction. Preserve root workspace, crate identities, public/wire/IPC semantics, dependency and feature/profile boundaries, generated bindings, distribution and data migration contracts. Preserve D23 (artifact antivirus retired without replacement), all remaining signing, attestation, supply-chain and release authority. No production secrets, signing, release, deployment, physical input or ReceiveOnly qualification absent separate explicit owner authorization. Mock/PlanOnly is not native proof. No forced input, process tampering or changes to game interaction.

## 2. Evidence and problem ledger

All paths below are relative to the research baseline; verify current ownership and exact selectors before editing. F = finding, not a blanket verdict on all tests.

| ID | Evidence | Status and design implication |
| --- | --- | --- |
| F01 | `crates/sky_player/src/engine/tests/supervisor_retirement.rs` around 650–699 and 972–1003: sleep 20 ms before contention, 50 ms elapsed assertion | Confirmed causal gap. Establish lock ownership and observe completion before unlocking; watchdog only. |
| F02 | `startup_admission.rs` around 1293; native desktop `native_runtime/tests/playback_integration/terminal_release.rs` around 988, 1004, 1173 | Fixed waits and negative assertions need proof the decision/cycle executed. Some waits intentionally test elapsed-time behavior; adjudicate individually. |
| F03 | `apps/desktop/tests/e2e/player-dock-density.spec.ts:114–137`; [failed job](https://github.com/pumni/Sky-Auto-Player/actions/runs/46104910120/job/140855631080), expected delta <=1px, got 124px | Real historical failure; current source unchanged. Root cause unproven. Reproduce on original/current source, inspect trace/layout before selecting a remedy. |
| F04 | `xtask/src/checks/bindings.rs:126–155`, `checks.rs` desktop-native and workspace test routes | Export invokes unfiltered shell library tests. Feature configurations differ; isolate export without losing non-export coverage. |
| F05 | `checks.rs` run_rust_group sets RUST_TEST_THREADS=1 | Workspace-wide harness serialization confirmed; resource-specific need and actual savings not yet measured. |
| F06 | `xtask/src/process.rs:82–109` | Successful captured output discarded; no per-call deadline on this API. Existing owned/bounded/filtered APIs must be preserved. |
| F07 | `xtask/src/checks/workflow.rs`, `ci_contracts.rs`, release-related checks and audits | Handwritten indentation/text matching and private identifiers couple some checks to spelling rather than semantics. Individual replacement requires equal or stronger proof. |
| F08 | `checks.rs` SKY_DESKTOP_SKIP_BROWSER checks presence, all can print PASS despite skips | Explicit skip semantics and coverage reporting need correction; existing leaf/JSON results are useful. |
| F09 | `bindings.rs:187–197`, WP3 and WP6 evidence | Fixed export directory; concurrent same-checkout invocation unsafe, ordinary exports have polluted checkout. Separate outputs or enforce explicit exclusion. |
| F10 | `checks.rs` inventory, runner and pointers; xtask responsibilities | Some source organization already improved by WP4–WP6. No new broad file-splitting program; simplify only demonstrated duplication or responsibility overlap. |
| F11 | [Windows Rust log](https://github.com/pumni/Sky-Auto-Player/actions/runs/46143176593/job/140973573134) | Latest rustfmt Unix newline policy conflicts with Windows checkout. |
| F12 | [Cargo Audit log](https://github.com/pumni/Sky-Auto-Player/actions/runs/46143176593/job/140973573182) | Pinned cargo-audit rejects [output] missing quiet; inspect full pinned schema, not just first error. |

Earlier green [run 46140714391](https://github.com/pumni/Sky-Auto-Player/actions/runs/46140714391) at bffefa92 is comparator only. Research HEAD run 46143176593 failed. Neither proves future HEAD readiness.

## 3. Binding design decisions

D01 — Test by contract: pure unit/policy tests use explicit inputs, timestamps, deterministic fixtures; state-machine tests use controlled events; worker/IPC integration use actual production paths with observable acknowledgements; browser tests cover user-observable contracts; packaged tests qualify actual package/update paths; native timing qualification stays real QPC/OS and separately authorized. One primary owner per invariant; upper layers test incremental integration risks, not exhaustive duplicate cases. Do not delete a test solely because it looks similar.

D02 — Synchronization: never use sleep or elapsed time to infer readiness, lock ownership, processing or completion. Bounded polling of a relevant predicate is acceptable if failure is explicit and observations are meaningful. For no-effect assertions establish the stimulus was actually processed; for repeated-edge assertions prove relevant cycles occurred. Use channels, barriers, acknowledgements or existing checkpoints; establish causal order, verify before releasing blocker. Timeouts are watchdogs, not proofs. Cleanup (release, stop, join, restore) on every path including panic/error. Real-duration tests remain only when timer/expiry/native scheduling is itself the contract, with stated rationale and a separate deterministic policy test when appropriate. No global fake-clock plumbing or production hot-path changes purely for tests.

D03 — Browser tests: use semantic locator or deliberate stable test ID; observe correct state, related geometry in one coherent browser snapshot, retry the final-state assertion only if eventual stability is the actual contract. Do not mask flicker or persistent layout errors by retries; preserve 1px invariant unless leader authorizes a product contract correction. Test expected responsive breakpoint and intentional animations separately. Capture first failure trace, screenshot, computed layout and viewport; no unconditional sleeps, increased timeouts/retries, broad skips or screenshot refresh as cure. Preserve isolated owned server and cleanup.

D04 — Bindings: a generation command generates only bindings; a read-only drift check compares exact files to tracked output, including missing/extra/stale files and command names. Use existing ts-rs export selectors if supported by pinned version; otherwise propose narrow explicit generation entry point. No new framework. Separate behavioral feature-matrix coverage from generation. The 93 previously retained blobs and all public wire identifiers must match unless owner separately approves product change. Every removed test invocation must have coverage mapping and evidence. Test-owned temporary outputs, no accidental tracked writes or untracked files outside ignored owned paths, no broad cleanup. Concurrent read-only runs must isolate outputs or fail early explicitly before writing. Explicit generation may update only its owned checked-in outputs.

D05 — Isolation: classify tests by shared resource (process env, OS/user singleton, mock foreground, global fault hooks, filesystem, ports, timers, generated files, workers). Restore via scoped ownership/RAII, use per-fixture temporary data, no global serial policy for unrelated pure tests once proven safe. Selective serialization for real resource conflicts is allowed. Do not adopt nextest, new runtime, runner or dependency without a leader decision informed by benchmark, compatibility, doctest/feature inventory and cost. Parallel execution is not a prerequisite for source acceptance if actual isolation would be unsafe; document retained exceptions.

D06 — Runner: separate streaming non-sensitive commands from structured capture and owned bounded subprocesses. Preserve exact argv/cwd, exit status, signal/timeout, containment, secret filtering and redaction. Stream Cargo/Clippy/test/browser progress and retain useful raw output; no unconditional buffering of unbounded output, no fake heartbeat, no hidden retry. A hung command must fail with useful diagnostics and clean up its owned resources; never kill unrelated processes. Preserve release/credential child environment restrictions; no generic streaming of secret-bearing commands. Use bounded output only where appropriate; do not replace sound process APIs just for uniformity.

D07 — Coverage: distinguish passed, failed, skipped, not-run, partial coverage, reused valid proof and unknown source. Explicit bool values, reject invalid skip configuration or fail closed as appropriate. An intentional skip is not proof of external success. Maintain machine-consumer compatibility or migrate all consumers atomically with reviewed schema/version; preserve exit semantics for required checks. Exact source SHA/tree and dirty status before/after, feature set and actual selectors, command/exit/duration. Never claim merge readiness from local PASS. No new generalized runner/registry unless leader explicitly approves concrete reduction of duplication.

D08 — Semantic guards: enforce manifest/feature graph with Cargo metadata or structured manifests; workflow syntax with actionlint; permissions, needs and conditions with structured configuration and independent fixtures where feasible. Actual argv, runtime outcome, byte binding and failure paths for executable contracts. Private name, whitespace and substring markers are not behavioral proof. Retain safety-critical lexical guards until equivalent/stronger replacement is demonstrated. A structured parser is not a GitHub expression/shell interpreter; do not pretend it proves execution. Preserve fail-closed unknown diff classification, exact source/run/attempt and all ten remaining CI lanes. No wholesale deletion or generic policy engine. Test valid harmless variations and meaningful invalid mutations, not simply the same literal used by checker.

D09 — Toolchain: keep existing pins and policy; align Git checkout with rustfmt LF for Rust files (verify CRLF historical byte fixtures unchanged). Validate complete pinned Cargo Audit configuration via actual invocation; preserve audit scope, freshness and no advisory suppression. Latest CI failures are baseline repairs, not authorization to lower gates. No enablement of tentative cargo-deny gate or arbitrary lint additions.

D10 — Scoped context: root AGENTS stays concise. Add narrowly scoped guidance for playback tests, browser tests and xtask when owning fixes demonstrate need; native desktop scope if inventory shows distinct rules. No duplicate contracts, Skills catalog, orchestration or AGENTS everywhere. See plan for candidate content. At completion merge durable rules into owning docs and archive this SPEC/plan via immutable links in HISTORY, remove active temporary work-order files/links.

## 4. Acceptance requirements

A01: independently reproducible exact-source baseline and final inventory by package/target/feature and test name, including ignored, doctests, fixture-only and export selectors. Every deletion/rename/replacement has an explicit old-to-new invariant mapping, not merely total count. Report existing default-only shell test limitation separately; do not silently claim supported feature matrix wider than contract.

A02: F01 lock regression is detected by a disposable negative control which actually waits for the held lock, and cleanup succeeds on both outcomes. F02 negative assertions fail under deliberate wrong decision/duplicate edge with processed stimulus. No wall-clock proof of readiness in migrated tests. Targeted negative controls are disposable, never pushed as live product mutations.

A03: F03 reproduce/diagnose with trace or classify not reproduced using repeated exact-source runs; test meaningful invalid layout fails and expected layout passes. Preserve user-visible contract and failure evidence. No speculative UI refactor.

A04: binding export and drift checks match baseline generated bytes and selectors; missing/extra/stale files detected; ordinary test and check runs do not dirty checkout; isolated/concurrent invocation behavior proven. Explicit matrix covers prior no-default tauri-test and all-features unique behaviors, plus any other required configuration.

A05: isolation and feedback measured on comparable machine, toolchain, cache, worker count and workload. No arbitrary speedup quota. Record failures, all samples, cold/warm distinction, medians and range; no invented p95 from tiny samples. Report net cost (including build and subprocesses). Demonstrate safe parallel candidates and retained serial rationale.

A06: runner tests cover success, nonzero, spawn error, large output, timeout/cleanup, partial/missing/duplicate leaves, invalid skip and source mismatch. Verify consumers, secrets and failure status; use fake credentials only in approved disposable fixtures. No changes to release authority.

A07: every changed/retired guard has invariant, original coverage, replacement oracle, negative case, valid variation and remaining limit. Classifier tests include real Git deletion/rename, unknown/malformed diffs, union of lanes; hosted aggregate still fails on mandatory lane failure/missing evidence. Preserve signing, NSIS/updater, SBOM, provenance, supply-chain gates and all unaffected negative tests.

A08: final pinned format/lint, targeted and canonical local checks, relevant website checks, release self-test and feature/packaging checks if affected; full required exact-final-HEAD hosted status. Local and source acceptance independent of hosted pending; no merge until final gates and owner approval. Final source identity clean, no dead private path, unused helper/dependency, transitional alias, stale selector, obsolete docs or compatibility implementation. Historical immutable evidence is not dead code.

## 5. Reference designs

- [Codex scoped synchronization guidance](https://github.com/openai/codex/blob/8cae463a69aced7705440acaaa9c9cba2d36e056/codex-rs/code-mode-host/AGENTS.md)
- [OpenCode test guidance](https://github.com/anomalyco/opencode/blob/3436a8e928d0e1fd519ede2e421602db896e0622/.opencode/skills/unit-tests/SKILL.md)
- [Cargo test](https://doc.rust-lang.org/cargo/commands/cargo-test.html), [nextest groups](https://nexte.st/docs/configuration/test-groups/)
- [Playwright practices](https://playwright.dev/docs/best-practices), [assertions](https://playwright.dev/docs/test-assertions), [clock](https://playwright.dev/docs/clock)
- [ts-rs export](https://docs.rs/ts-rs/latest/ts_rs/), [actionlint](https://github.com/rhysd/actionlint)

These references inform the design, not authority to install tools or change the pinned versions. Inspect pinned behavior in the actual checkout.

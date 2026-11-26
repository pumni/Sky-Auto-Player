# Hold-only dispatch simplification — leader work order

Status: W1 AUTHORIZED; W2–W4 LOCKED pending leader review. This is a temporary, branch-scoped work order, not a second permanent architecture contract.
Repository: pumni/Sky-Auto-Player.
Single implementation branch: `refactor/hold-only-dispatch`.
Immutable comparison baseline B0: `a76fc2ad2eb29b56857a380799045a556cbdb59c` (main at planning time).
Toolchain: pinned Rust 1.99.1, Windows 11 x86_64-pc-windows-msvc, existing Bun pin and exact Cargo.lock.
Owner: repository owner. Design/review authority: leader. Implementation: user's CLI agent.

## 1. Mandate and decision rights

Goal: reduce mechanisms, runtime work, and maintenance burden while preserving musical correctness, Windows safety, and measurable timing/resource behavior. Not a rewrite and not a syscall speed contest. Work in large end-to-end packages; no per-file approval ceremony.
- Leader sets contracts, architecture, experimental design, thresholds, scope and final decisions. CLI may choose local idiomatic implementation details, names and test organization only; it must not select policies, relax assertions, add dependencies, migrate public wire/config contracts, change input authority or select a production winner without written leader authorization.
- Read root and scoped AGENTS.md, docs/INDEX.md, docs/playback-engine.md, docs/playback-verification.md, docs/ci-validation-topology.md, SECURITY.md, CONTRIBUTING.md and relevant actual code/tests. Resolve mismatches by reporting them; do not silently alter contracts. Work order adds scope, never overrides safety.
- User authorizes work and pushes on this one branch; no force push, no rebase of published history, no main edits, no merge, release, tags, deployment, secrets, CI policy/gate changes, extra feature branches or unrelated cleanups. Detached baseline worktrees allowed solely for reproducible comparison; not additional implementation branches.
- One draft PR is coordination channel. Push coherent milestone commits as soon as local work and evidence are ready; do not wait for CI before requesting leader review. CI is not an approval and leader review does not waive final CI requirements. Report failed/skipped gates truthfully, then request review. Owner returns to leader with PR URL and exact pushed HEAD; leader reads diff, evidence and current source, posts verdict and next work package. Do not start locked work. No autonomous background review promise.
- If blocked by a consequential choice, record alternatives, measured facts, affected invariant and recommendation. Continue independent authorized tasks. Stop at package boundary with explicit BLOCKED status; do not invent a decision. Fix introduced defects within authorized scope before handoff unless blocked.

## 2. Frozen semantics (all work packages)

1. Hold-only. No G, cooldown, minimum Up-to-Down spacing, hidden delay, or extra same-key timing gate. Existing nominal Up-gap advisory is not a runtime gate; W3 may remove it only if leader authorizes. Do not add a user setting. No game-specific claim about polling/receiver visibility.
2. H is a minimum musical sender-side hold: successful Down completion QPC + H ticks (ceiling conversion); musical Up pre-call cannot occur before that. Safety release may violate H intentionally. Preserve existing materialization formula, defaults, bounds and rounding in W1/W2; no integer rewrite now (would change fixtures). Absolute authored onset and epoch preserved; no early Down, no adaptive hold, no backdating, no drop, tempo adjustment, reorder, or implicit rebase. Resume existing epoch policy unchanged.
3. Single writer, fixed 15-slot executor, onset-only immutable plan, same-key nominal validation interval >= H, independent due Ups, intact next chord, strict cursor ordering. For same-key repeats require Up commit before next Down; no overlapping up/down masks or same-key batch retrigger. Disjoint due Ups can share packet with eligible Down, all Ups first, existing materialized slot/order preserved. No waiting intentionally for batching. No new heap/async runtime/threads per key.
4. Preserve public commands, DTO/schema, persisted settings, telemetry fields/semantics and historical fixtures unless explicitly authorized later; avoid renaming public identifiers for aesthetics. Preserve existing strict vs production diagnostic behavior.
5. Keep Windows SendInput scan-code only, QPC typed clocks, immutable HWND/PID/process incarnation retained handle, fresh final admission, modifier semantics (mixed can fall back to independent Up-only with fresh admission), supervisor lease, control precedence, target generation, focus loss, power/suspend, RAII and journal-before-syscall/deferred commit/worker join fence. No broadened rights, hooks, game process observation beyond approved boundary, driver, PostMessage, or anti-cheat bypass. Do not imply foreground query atomically binds SendInput destination.
6. Exactly one raw attempt for musical packet; zero, partial, integrity and timing anomalies retain current outcome/consumption/cleanup rules; no Down replay. Maintain distinction between scheduler and physical obligation state. Terminal safety Up can proceed without QPC when contract allows; bounded cleanup scope, background budget, independent observation and quarantine/recovery semantics unchanged. Sender return count != receiver consumption; GetAsyncKeyState != hardware ownership.
7. Allocation-free healthy dispatch remains mandatory. Interruptible wait (timer + event), cancellation/generation races and fresh QPC wake re-evaluation must stay correct. Clean pause no periodic timer wakes. No real-time priority, registry/HPET/affinity changes or new timer-resolution calls.
8. No changes to signing/updater/security policy, lockfile, dependency versions, CI, historical qualification profile manifests/limits. No weakening test or oracle, hiding dead code with allow attributes, removing test coverage as substitute for fixing code, or introducing unused scaffolding.

## 3. Work packages and leader gates

| ID | Whole vertical slice | State |
|---|---|---|
| W1 | Baseline characterization + direct deadline dispatch migration, tests, callers, docs and deletion audit; comparable nonphysical evidence | AUTHORIZED NOW |
| W2 | Fixed spin versus startup calibration / waiter resource study and production simplification | LOCKED |
| W3 | Separate scheduler correctness from diagnostic projection; offline H/slack/HOL advisory and consumer migration | LOCKED |
| W4 | Full integration, remaining justified simplifications, release-readiness evidence and cleanup | LOCKED |

These are packages, not file limits. W1 should touch all necessary native engine, tests, adapter consumers, scripts and owning docs end-to-end. Do not wait for per-file approval. W2–W4 below establish direction, not permission to implement ahead.

## 4. W1 — authorized detailed execution

### W1a baseline and measurement setup

- Fetch origin; verify B0 and branch HEAD. Never switch a dirty user's worktree destructively. Record preexisting failures separately. Read source: sky_dispatch_core/src/executor.rs (evaluate_at/on_send_completed_at), sky_player/src/engine/worker/dispatch_loop.rs, dispatch_loop_scheduler.rs, wait.rs, admission.rs, cleanup, observer/health paths, sky_dispatch_win32/src/input/packet.rs, tracked/packet_send.rs, event and wait/hybrid.rs. Trace production, dry-run, mocks and qualification callers; inspect tests/feature gates, scripts, UI consumers for changed symbols. Map all staging-dependent behaviors including tests.
- Build/run B0 and candidate with same pinned toolchain, lock, features, profile and workload, separate clean detached worktrees/output directories. Record exact source SHA/tree, Cargo.lock SHA256, rustc -Vv, build command/profile, binary hash when relevant, Windows build/CPU/logical cores/power state (no serials or personal identifiers), harness revision and run order. Do not rebuild baseline with candidate's code unless explicitly a common external test-only harness; label methodology. Preserve baseline evidence as revision-bound.
- Existing ReceiveOnly historical profiles have frozen identities and branch constraints. Do not modify old profiles or pretend they qualify a new branch. Reuse compatible nonphysical harness and baseline adapter, or add a narrowly scoped test/benchmark runner for this work order only. No second production dispatch implementation or runtime switch. If current harness cannot support fair comparison, report exact blocker; use deterministic parity evidence and nonphysical benchmark where possible.
- Baseline capture is part of W1, not a separate review gate. No physical SendInput yet (see section 8).

### W1b target algorithm — direct evaluation after wake

Current implementation speculatively evaluates a future deadline, stages packet bytes, then reselects and compares/rebuilds after actual wake. Replace production speculative staging/reselection with one direct decision path. Do not preserve old path as a toggle, fallback, legacy wrapper or parallel engine; B0 Git commit is control.

Algorithm (conceptual, integrate with real control loop):
1. Process lifecycle control/target changes and existing modifier retry deadlines; sample QPC. Evaluate executor at actual now. If Finished, follow existing drain/retire contract; if Wait, wait interruptibly on earliest musical/control deadline with existing production waiter policy. No speculative evaluate_at(future deadline) and no prepared packet bytes carried across a future wait.
2. On Due use authoritative observed QPC (fresh sample where contract requires); re-evaluate current eligibility. Interrupted/Early never send stale work; process lifecycle/replan and recalculate. Recheck all existing target/epoch/control semantics; an overdue independent Up must not wait for blocked Down. Preserve wait provenance for diagnostics, not cached payload. If not yet eligible, wait again. Avoid repeated zero-duration waits/spin when chord held or modifier blocked.
3. On Send build one current packet on stack from due up mask + complete eligible next chord, with existing prepared INPUT templates, key profile and attribution. Capture target stamp for this immediate attempt. Run fresh final packet admission. Preserve mixed modifier rejection -> borrowed initialized Up prefix -> fresh Up-only admission, modifier hold budget, independent Up eligibility, and error outcomes. Retain local candidate/view only within the dispatch attempt if needed; do not store across wait. Preserve fresh proof/commit checks, no new physical writer.
4. Journal in flight -> one SendInput -> preserve count/error/QPC evidence -> commit scheduler and journal using existing transaction semantics. On anomaly terminate musical playback with existing exact obligation cleanup. Handle already due work without arming a needless timer. Keep diagnostic output semantics in W1; no telemetry simplification beyond removal of staging-specific private plumbing. Do not change spin policy or H formula.
5. Re-evaluate after actual control transition or successful commit; no carry-forward stale deadline/epoch. Existing observer wake evidence must be attributed to the actual packet and never fabricated. Existing pure executor remains canonical for plan simulator and worker.

Implementation scope explicitly includes all call sites, test-support adapters, tests, fixtures and docs that refer to removed speculative staging mechanism. Keep packet preparation types needed for immediate safe FFI and Up-prefix; do not mechanically delete by name. Preserve necessary pre-wait control/target handling even if speculative packet admission disappears. Replace stage-specific assertions with equivalent or stronger observable assertions (eligible masks, exact send count, owner/control precedence, actual wake, order, allocation); historical revision-bound fixtures stay intact. Record before/after function call graph and why every surviving helper is needed.

### W1c verification matrix (must run, report results)

Deterministic tests on actual production worker orchestration via mocks/test-support, not just helper functions:
- Exact/early/late wake; late wake adds independent Ups to disjoint chord; overlapping held chord blocked even if its Up due, Up commits then next Down; no overlap packet, no early Down, no early musical Up. Multiple overdue independent Ups, same-time disjoint mixed, 1/5/15 width, same-key I=H and dense repetitions, sparse/end-of-song, near-overflow arithmetic, QPC nonintegral conversion.
- Control injection at before wait, during wait, due return, after evaluation, before final sender (pause, quit, skip, panic, lease, target change, focus, suspend); verify no forbidden new Down and cleanup scope. Modifier-held Down-only and mixed Up fallback, modifier timeout unaffected, independent Up deadlines retained. Paused idle wake count.
- Zero/proven raw-zero, partial/integrity anomaly, failed pre/post QPC, panic after journal before syscall / after syscall before commit, journal reconciliation, owner mismatch, recovery after join. Assert generation/cursor/accounting and no duplicate Down. Use existing injected seams; no real keyboard in W1.
- Observer on/off and production vs strict modes: preserve event counts/attribution, missing/dropped evidence accounting; allocation test with observer off and applicable observer modes. Existing no-allocation test must still exercise shipping worker (not only pure evaluator).
- Differential trace: deterministic same clock, controls, mocked transport outcomes and costs for B0 vs candidate; compare normalized ordered packet masks, attempts, timestamps at actual boundary, generation/obligation state, terminal reason and lifecycle. Expected differences only scheduler evaluation count, packet builds and staging-internal markers; timing differences from reduced modeled computation must be separately explained, never silently ignored. Test late wakes and adversarial control ordering, not just happy case.
- Baseline-vs-candidate nonphysical measurement: measure scheduler evaluation count, packet build count, wait-arm count, allocations, CPU time/operation and wall time where meaningful. Include 1/5/15 width, sparse cadence 100 ms, dense repeated same-key I=H and H+1 ms, mixed disjoint, late wake bursts, long-plan 100/1k/10k chords. At least 6 alternating paired runs per feasible benchmark workload (AB/BA), warmup excluded equally, report all runs, median, p95/p99/max where sample count supports it, sample counts and dispersion; no trimming bad runs except predeclared environmental invalidation recorded. Separate observer-off production and diagnostic overhead; report mock vs real clock. Do not present simulated sends as Windows SendInput latency. If fixture cannot cover scenario, add focused deterministic test and report benchmark gap rather than make up data.
- Evidence-driven acceptance (leader decides): no functional regression, no new allocation, no extra raw attempts or periodic wakes, no production duplicate scheduler path. Candidate should reduce evaluations/builds on future waits and simplify source/control flow. Nonphysical CPU/wall regression >5% median across repeated paired runs or >10% p99 where meaningful is an investigation flag, not an automatic waiver/pass/fail; noise and absolute differences reported. Native timing remains NOT RUN until approved. Do not choose a winner based on a single timing run.

### W1d cleanup/deletion acceptance

Audit all changed crates, desktop adapters, tests, docs, scripts and cfg/features:
- Delete replaced future evaluation, staging/reselection reuse comparisons, stale cache-only fields, private helpers, staging-specific test seam counters no longer needed, imports, unused structs/functions/constants, obsolete tests/fixtures not historical, unneeded cfg branches and doc claims. Rename surviving candidate abstraction to immediate-attempt meaning if beneficial; no wrapper alias preserving retired private APIs.
- No runtime feature flag retaining old implementation. Test-only benchmark variants must be narrowly scoped, used by tests and explicitly temporary; delete before final W4 unless leader authorizes permanent harness with actual consumer. Historical evidence stays immutable in Git, not copied into live code.
- Search both deleted symbols and related semantic patterns via rg/git grep across entire tracked tree. Report each removed mechanism, old symbols, migrated consumers, exact search and residual references with reason. Review call graph and feature combinations; no warnings alone is insufficient for pub/dead semantic code. No new #[allow(dead_code)], blanket allow, cfg hacks, ignored tests, or disabled assertions. Existing allowances not expanded.
- Update docs/playback-engine.md and docs/playback-verification.md where behavior/verification changed; keep them canonical. Work order is not permanent contract. Update existing tests instead of duplicate old/new test suites where sensible; preserve independent oracles. Do not archive/delete this active plan until leader closes W4; archive via immutable Git link in HISTORY.md per docs/AGENTS.md.

## 5. Commands and local checks (all from repo root unless noted)

Use pinned wrapper; follow repository's actual xtask and CI topology. Capture exact commands, exit codes and logs. If command unsupported by checkout, report and use documented equivalent, not an unpinned compiler. Do not change gate definitions.
```powershell
node scripts/toolchain-pin.mjs verify rust
node scripts/toolchain-pin.mjs exec -- rustc -Vv
node scripts/toolchain-pin.mjs exec -- cargo fmt --manifest-path rust/Cargo.toml --all
node scripts/toolchain-pin.mjs exec -- cargo fmt --manifest-path rust/Cargo.toml --all -- --check
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc -- --test-threads=1
node scripts/toolchain-pin.mjs exec -- cargo check --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance
node scripts/toolchain-pin.mjs exec -- cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets -- -D warnings
node scripts/toolchain-pin.mjs exec -- cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance -- -D warnings
node scripts/toolchain-pin.mjs exec -- cargo xtask check rust
node scripts/toolchain-pin.mjs exec -- cargo xtask check static --skip-supply-chain
git diff --check
```
Before running tests inspect suites for real input; do not execute any real-input/ignored/native qualification case without explicit machine/receiver/scope approval. Build/check of real-input-acceptance feature allowed; not execution. Run focused additional tests (including test-support configuration) and affected desktop native tests when adapters changed:
```powershell
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- --test-threads=1
node scripts/toolchain-pin.mjs exec -- cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- -D warnings
```
If desktop TS changed, use pinned Bun from apps/desktop, frozen install and bun run check, plus affected e2e. No blanket requirement to rerun unchanged costly gates at every iteration. Final W4 merge readiness uses pinned `cargo xtask check all` on final relevant source and required CI at exact PR HEAD; review starts immediately on push without waiting for CI. Never merge based on local evidence alone.

## 6. Locked W2–W4 — future leader work orders

W2: isolate wait policy experiment from direct path by using accepted W1 as control. Compare fixed 0/250/500/1000 us versus existing per-session startup probe (6 samples, p99=max, 50 us margin, 250–1000 clamp). Freeze threshold for a session. Hold, worker priority and sender policy unchanged during threshold comparison; MMCSS off/normal/high only as separately approved experiment. Capture ready delay, wake error, sender/receiver CPU, spin time, wait calls, interruption latency, startup/preroll and measured tails. No real input until section 8 approval. Select production policy only on leader decision, then delete superseded production calibration path and obsolete metrics/contracts with explicit migration authorization if needed. Existing tests or evidence profile schemas remain pinned. No arbitrary hard affinity/priority or timer-resolution changes.

W3: keep scheduling semantics and transport invariants; separate correctness data (mask, cursor, release deadline, packet timing eligibility) from diagnostic attribution/nominal release math, including observer, strict checks and public projection. Investigate precompute vs lazy observer projection, compare memory at 100/1k/10k/large plans. Existing nominal Up-gap advisory may be replaced by repeat scheduling slack I-H if explicitly approved; not a gap constraint. Offline model uses canonical executor; examine ideal/empirical/stress cost assumptions, distinguish H displacement, true next-chord blocking and worker occupancy, do not claim measured performance from hard-coded costs. No production change to H rounding without separate leader decision. Admission/lease consolidation only if proof and authority/control ordering unchanged; no relaxation of fresh checks or release semantics.

W4: end-to-end integration plus final deletion audit; only authorized refactors. Verify exact source and baseline, reproducible native and nonphysical evidence (or mark limitations), all local gate requirements, docs and UI consumers, review PR changes and security audit. No merge until leader and owner approve; required CI still applies. Archive this work order as immutable history, remove transient experimental code/evidence from live product; no abandoned code paths. Final merge authority remains owner.

## 7. Handoff format (each package, same branch + same PR)

Push only relevant code/docs/tests and a concise revision-bound evidence manifest/report (not large traces, binaries, personal info or secrets). Follow docs/AGENTS.md for ignored outputs/CI artifacts; use PR comment with results and links, compact machine-readable CSV/JSON summary when needed and owner-approved artifact location. Keep raw logs outside checkout; if artifacts inaccessible to leader, paste aggregate and representative raw samples with SHA256 and explain limitation. Do not claim unavailable raw logs were reviewed.
1. Package/status READY_FOR_LEADER_REVIEW or BLOCKED; exact baseline/candidate SHA/tree, branch, commits, clean status, build identifiers, and exact PR URL.
2. Architecture before/after, invariants covered, changed files grouped by subsystem, deleted symbols/mechanisms and residual-reference audit, test changes and independent oracle.
3. Commands with exit codes and pass/fail/skipped; failure excerpts and preexisting vs introduced classification; evidence source and run counts. CI state informational only, no waiting.
4. Benchmark matrix raw paired summary, p50/p95/p99/max with N, CPU/alloc/waits, build/OS provenance, artifacts/links/hashes, missing measurements explicitly marked. No inferred receiver correctness from mock or sender counts.
5. Blockers/decisions required, alternatives and recommended option (not chosen); risks and next requested package. No self-approval.
Leader reviews pushed exact SHA, compares B0/last accepted commit, reads source and consumers not just summary, checks removed paths and test independence, posts ACCEPT / CHANGES_REQUIRED / BLOCKED with concrete rationale. New push invalidates old acceptance for changed code. W1 pass is not native qualification or merge approval.

## 8. Native safety and evidence boundary

Per SECURITY.md and docs/playback-verification.md, explicit owner approval of exact Windows machine, project-owned ReceiveOnly receiver and allowed instrument keys/scope is required BEFORE any physical-input test. The general request to benchmark does not satisfy all runbook details. W1 authorized only mock, dry-run, PlanOnly and nonphysical work. Prepare plan/manifest and list missing approvals; do not pass -OwnerApprovedReceiveOnlyRun yet. No real game as qualification receiver, no game hooks or memory reads, no keyboard capture outside controlled receiver, no automated gameplay benchmark. Use exact verified receiver identity/foreground and existing safety rules; abort on unexpected target/input/ownership. Distinguish native input stream acceptance from game behavior. WPR/WPA tracing only under owner-approved scope, not required to complete W1 nonphysical work.

## 9. Technical basis (external references)

- Microsoft SendInput contract: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput — serial insertion, return count and UIPI, not receiver acknowledgement.
- Keyboard input overview: https://learn.microsoft.com/en-us/windows/win32/inputdev/about-keyboard-input — state and routing, no reset of existing keyboard state.
- Windows Performance Analyzer CPU analysis: https://learn.microsoft.com/en-us/windows-hardware/test/wpt/cpu-analysis — distinguish ready/wait/running causes instead of assuming timer expiry equals execution.
- Clippy usage: https://doc.rust-lang.org/clippy/usage.html — warnings-as-errors includes rustc dead_code; still perform consumer/semantic deletion audit.

## 10. Immediate CLI instruction

Read this entire work order; implement W1a–W1d as one complete end-to-end package, run authorized local checks and nonphysical benchmarks, push coherent commits on refactor/hold-only-dispatch and post a single handoff on the draft PR. Do not wait for CI. Do not start W2–W4 or choose architecture changes outside this contract. Report blocked native authorization rather than manufacturing evidence. No self-merge.

# Completion-Anchored Pulse Engine — leader work order

Status: ACTIVE — leader-approved scope; implementation authorization: package A only until review.
Repository: pumni/Sky-Auto-Player
Branch: refactor/completion-anchored-pulse-engine
Baseline: 1901cff6e2db3617b57145c7eae8d44a6c877fa2 (tree c15bcacb45245f1a29f9a4a7043dbdf33839a468)
Owner: repository owner; architecture/review authority: ChatGPT leader; implementation: local CLI agent.
This is a temporary branch work order, NOT a second permanent architecture contract. Update the existing owning docs with implementation. Archive this work order by immutable Git link in docs/HISTORY.md and remove the live work-order file only when the leader approves final closure.

## 1. Objective and scope

Replace the fixed-pulse production Down/Up authored action pipeline with a single onset-only chord plan and completion-anchored release scheduler. Simplify the entire path, not just a few files. Preserve musical fidelity, external contracts and security; seek lower complexity and bounded resource cost, not an unproven latency claim. Owner reports reliable note detection at 60 FPS with 16,667 us hold and rapidly worsening loss below that; not proof of exact Sky sampling API or guarantee at other FPS. G was deliberately removed because near-minimum retriggers are rare; do not reintroduce it.

Authoritative references: AGENTS.md, SECURITY.md, CONTRIBUTING.md, docs/INDEX.md, docs/song-planning.md, docs/playback-engine.md, docs/playback-verification.md, docs/architecture.md, docs/desktop-workbench.md, docs/desktop-type-boundary.md, docs/rust-toolchain-policy.md, docs/ci-validation-topology.md. Read relevant scoped guidance. Baseline source is evidence, not a reason to preserve obsolete private architecture. Microsoft API references:
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps
- https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw
- https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-setwaitabletimerex

Non-goals: new input API, game memory/hooks/automation beyond security policy, G/cooldown or post-cleanup time floor, frame phase tracking, adaptive hold/spin, custom drivers, extra realtime writers, new user-facing controls, tempo changes, new dependencies, opportunistic website/updater/release/CI changes. No default priority/spin/timing change in this refactor. No new severe-lateness stop threshold: retain baseline late-onset semantics and measure; any new drop/abort/reorder policy needs leader decision. Do not add a fallback legacy engine or feature switch.

## 2. Decisions frozen by leader

D1. Fixed pulse only. Preserve existing settings and validation, including hold_frames options, FPS, margin, tempo, rounding and saved configuration. H_us = ceil(hold_frames * ceil(1e6/FPS)) + margin. At 60 FPS, 1 frame and zero margin: H=16667 us. Materialize once; convert H to QPC duration with checked ceil. Keep authored sustain as diagnostic only, per song-planning contract.

D2. Plan is immutable sorted unique chord onset groups in the QPC timeline, each with nonempty allowed 15-slot mask. One canonical representation for executable intent, not separate Down and Up arrays. Preserve same-time dedup/source provenance, loss semantics, stable canonical slot ordering, allowed key profile mapping, source IDs where consumed, memory limits, cancellation checkpoints, checked overflow and structured user-facing diagnostics. Source and scheduled onset distinctions survive. Do not silently merge distinct source onsets without existing conversion-loss reporting.

D3. For musical Down, D_i = epoch + onset_i; no early send (including first onset). For each key successfully transported in a healthy Down batch, release_not_before = authoritative Down completion QPC + H_ticks. A successful Up clears that active key immediately. No G, no minimum cleanup-to-resume delay, no authored Up floor in production. Preserve exact baseline rounding behavior where observable; if independent rounding makes old prepared Up > completion+H, report the counterexample and proposed disposition before changing behavior. Validate all sums, including schedule end and runtime overflow; use typed QPC, timeline and duration units. No floating point in hot path.

D4. Plan admission rejects same-key authored intervals < H_us (preserve typed error context and applicable UI risk behavior). At equality, allowed; Up must commit before re-Down. Short nominal release gap < configured frame interval is advisory only, calculated as next onset - (previous onset + H); no extra runtime rule. Advisory model uses production executor and explicit costs; report modeled HOL/hold displacement as diagnostics, never gameplay confirmation. Do not infer game acceptance from SendInput success.

D5. Strict linear chord order and intact chord. Per-key independent due releases. At each fresh evaluation, consider all due active Ups and only the next chord if due and ALL its slots idle. Combine disjoint due masks in one batch, Up before Down. If overlap with active slots, send due Up and re-evaluate only after commit; no Up+Down of same slot in one packet. Do not delay an eligible Up to batch with future Down, or split chords to improve latency. No early release for musical purposes. No replay on transport anomaly. Preserve baseline catch-up behavior and epoch except existing pause/resume rules.

D6. Scheduler owns only musical state: immutable chord slice, cursor, epoch (may remain worker-owned), active mask and per-key release deadline; minimal per-slot identity/attribution only if there is a real accounting or observation consumer. Keep per-generation accounting semantics, not necessarily old generation structs. Use stable generation identity derivable from chord index + slot or equivalent checked indexing; do not erase independent accounting of pending, activated, released and terminal/cancelled intent. No alloc on healthy dispatch. No binary heap or per-key timer/thread.

D7. Transport journal is separate, authoritative for uncertain physical effects. Keep one physical writer until join; exact origin identity, retained owner process, foreground, modifier, lease, control, suspend and cancellation contracts. Keep existing bounded cleanup and reconciliation, raw-zero distinction, partial/integrity/QPC failure behavior, needs_up vs needs_probe, cleanup probe authority, quarantine and worker-join handoff. Stop/Pause/terminal safety release bypasses musical H. Do not conflate successful Windows insertion with game reception. No weakening of safety to gain timing. No keyboard/input hooks, process memory, injection, debugger, anti-cheat bypass; SendInput only. No automatic physical tests.

D8. Keep existing absolute QPC epoch, high-resolution one-shot relative waitable timer + interrupt, frozen production pre-spin, priority/power acquisition behavior, preroll/lease/foreground grace and production admission. An expired/invalid control deadline may not create musical sends. Waiter wakes on control or deadline; stale staged candidates may not be sent. Do not change timer/clock backend merely to refactor. A later simplification may delete proven unused experimental paths only with evidence and leader approval. Keep current external settings, IPC DTO fields, telemetry schema and historical readers/qualification artifacts with live consumers; if a real wire change is unavoidable, escalate before implementation. Diagnostic authored Up target can be derived on demand from onset+H; it is not an execution gate. Never fabricate evidence or duplicate runtime state merely to preserve a private metric name.

D9. Schedule-end/empty-plan behavior and progress duration must match baseline, including the final pulse, trailing idle if applicable, stop/skip and zero physical work. Finish requires cursor exhausted, no active musical keys, and existing end condition; terminal success still requires clean journal, retirement and join. Keep pause cursor semantics: do not replay committed onset; drain on pause and revalidate authority before resume; no post-cleanup floor. Preserve pause/rebase time and terminal accounting exactly as baseline. If an existing external contract genuinely requires an independent authored Up or arbitrary duration, halt affected decision and bring a reachable production example to leader; do not silently narrow supported behavior.

## 3. Target boundaries and algorithms

sky_app_core: normalize/scale/map/dedup -> onset groups -> prepare-facing plan & advisory; no runtime Win32.
sky_dispatch_core: validated immutable onset/chord plan, checked QPC preparation, deterministic scheduler decisions and commit, generation accounting and test oracle. No direct OS calls.
sky_dispatch_win32: QPC/timer/interrupt, input profile & prepared INPUT payload, target authority, one SendInput primitive, journal/reconcile/cleanup.
sky_player: session preparation/arming, worker lifecycle, staging, final admission, sender transaction, metrics publication and terminal result.
Desktop shell: UI/IPC, prepare advisory, supervisor lease, retirement/quarantine and worker join fence. No second physical writer.

Representative private shape, not mandated identifier spellings:
    Chord { onset_ticks, key_mask, necessary attribution }
    PreparedPlan { chords, hold_ticks, end_offset, necessary memory/accounting }
    Executor { cursor, active_mask, release_deadline[15], necessary attribution/accounting }

Evaluation:
1. Read monotonic QPC; compute up_ready from active slots whose absolute release deadline <= now; track earliest future release.
2. Next chord target = checked epoch + offset; blocked if any active bit overlaps. Down eligible only if target <= now and no conflict.
3. If Up or Down eligible, return disjoint packet, authored Down target and independently attributable release eligibility. If blocked, do not repeatedly wake on an already due Down; use max(authored target, latest blocking release) as chord lower bound; next wake=min(that bound, earliest future active Up, relevant control deadlines). At release commit re-evaluate. Do not busy-loop on stale time or round to early send.
4. If none and exhausted, wait until preserved schedule end or finish as appropriate.
5. Stage packet before waiting where profitable; after wake/control/target drift, recompute or prove candidate equivalent, including newly due Ups and newly invalid Down. Up-prefix fallback when modifier blocks mixed packet requires fresh Up-only admission. No new preemptive SendInput path.
6. Journal in-flight before side effect, final authority and timing contract at actual boundary, exactly one musical syscall. On complete healthy result commit transport and musical state transactionally according to existing deferred journal contract. On any invalid/partial/zero result, prevent future musical Down, reconcile exact obligations, retain original evidence; no replay.
7. Down commit computes absolute release floor from completion; Up commit clears active state; update cursor and accounting only on accepted transition. Do checked arithmetic prior to mutation when possible; overflow after side effect goes through terminal cleanup.

Define useful independent deadlines and timestamp vocabulary in owning docs. Distinguish authored onset, eligible release, pre-call, completion, and optional diagnostic authored release. Never relabel sender observer as receiver acknowledgment.

## 4. Work packages (large end-to-end slices, one branch / one draft PR)

All packages may involve dozens of files across crates, shell, tests and tooling. No file-count cap; do not split into cosmetic micro-tasks. Build coherent commits; no forced merge/rebase/push to main, no force push. Do not proceed to next package before leader review explicitly authorizes it in the draft PR. While review/CI runs, report evidence; no waiting for CI green to request review. Local lint/format/test failures introduced by the package must be addressed, or transparently reported as blockers. Owner authorizes pushes to this branch only; CLI must not edit PR review decisions, this work order or release authority.

### Package A — End-to-end onset-only cutover (AUTHORIZED NOW)

A1. Inventory the actual call graph and consumers of authored actions/paired Up from song decoding/planning through compilation, QPC preparation, scheduler, desktop prepare and risk/advisory, worker boot/staging/dispatch/commit, acceptance scenarios, telemetry/evidence, generated bridge types, tests and docs. Include all supported formats and test-only builders. Inspect actual workspace and tests, not only docs. Record live wire/schema/historical identifiers and why retained.
A2. Introduce canonical validated onset plan and QPC preparation, migrate production callers end-to-end, implement D1-D6/D9 and migrate planner risk/advisory, duration, memory budgets and diagnostic attribution. Fixed-pulse has no paired authored Up action; derive nominal Up only at explicit external or diagnostic boundary if a real consumer requires it. Eliminate double materialization. Preserve physical key profiles and noncanonical mappings. Ensure no independent Up timing contract is lost.
A3. Replace executor implementation and integrate production worker/boot/commit without weakening journal/admission/lifecycle. Preserve external behavior and fault paths. Migrate all harnesses, benches, tests, acceptance workloads and shell callers to new plan. For historical immutable evidence, retain only readers/fixtures needed to read historical artifacts, not obsolete runtime code.
A4. Remove old runtime model/compiler/paired arena/unused Up metadata, old exports, helpers, adapters, tests tied only to removed private layouts and dependencies with no consumers. No parallel engine, no compatibility wrapper without a named external consumer. Use independent reference oracle, not a second production scheduler. Old algorithm may be represented in a small independent mathematical test oracle if necessary, not copied wholesale.
A5. Update owning docs for this cutover (song-planning, playback-engine, verification, architecture etc as applicable); do not update unrelated release/UI copy. Preserve schemas unless leader explicitly decides otherwise. Run package evidence in section 6. Submit one coherent checkpoint with implementation and deletion audit, exact SHA and coverage. Stop after push and notify leader; don't wait for hosted CI.
Exit: all production paths use onset plan; old executable paired schedule gone; tests establish behavior and invariants; all affected callers compile, lint/format; no unapproved public or semantic changes.

### Package B — Worker/lifecycle hot-path simplification (HOLD FOR LEADER)

Review staged scheduling, wait/interrupt, gate ordering, send transaction, metrics and cleanup path from session arm to retirement. Consolidate genuinely redundant private projections/checks, eliminate unconsumed metadata and unnecessary conversions/copies/re-evaluation while preserving race boundaries. Keep existing final admission, mixed Up prefix, lease semantics, terminal evidence and single-writer safety. Do not remove guards based on source aesthetics. Inventory calibration/experimental QoS/observer helpers; show call graph and evidence of nonuse first. Leader will decide removal scope and whether a calibration/priority change is justified. No target behavior changes without approval.
Exit: measurable or source-proven simplification, production-path safety tests and no-alloc evidence, no unused new/old paths, no unapproved policy changes.

### Package C — Full verification, qualification and closure (HOLD FOR LEADER)

Finish residual deletions, cross-repo consumer checks, independent regressions, documentation alignment and canonical check all. Compare baseline and candidate using reproducible same-environment nonphysical measurements; report absolute and relative memory, allocations, scheduler work, CPU/spin and timing where evidence exists. Native ReceiveOnly and game runs require separate explicit owner authorization and exact source-bound manifests; PlanOnly != physical qualification. Current historical qualification profiles/baselines/lock hashes remain immutable; if a fresh matched-baseline comparison requires new profile, leader decides before changes. Do not claim native performance or gameplay results if not run. Final PR review, then archive work order by immutable commit link in HISTORY and delete active work order. No merge/release by CLI.

## 5. Required independent verification

- Golden/reference trace for valid onset plans: chord order, event masks, exact one-attempt accounting, elapsed H, no early Down, no Up before completion+H, Up before re-Down at equality, no G. Compare with baseline traces at multiple QPC frequencies, including nonintegral tick conversions, first note, empty, last note, large time, tempo collapse, disjoint mixed batch and actual late completions. Explain differences, don't change expectations merely to pass.
- Planner: all sheet formats, duplicate source onset vs quantization losses, scan/profile validation, intervals H-1/H/H+1 and H+T-1/H+T, overflow and cancellation, duration and advisories. Distinguish impossible repeat from short nominal gap.
- Executor: 15-key chord, disjoint overlapping pulses, release independence under HOL, multiple simultaneous releases, blocked chord waiting without spin, no batching delay, late wait/replan, independent QPC H floor, idle/finish and pause/resume, generation accounting invariants. Property/fuzz tests with independent oracle and deterministic seed; no circular oracle using candidate evaluate.
- Production dispatch harness: early/late timer, stale staging, newly due Up, mixed modifier Up-prefix, stop/skip/panic/suspend/focus loss/owner mismatch/lease expiry, raw zero and partial/mismatched counts, QPC failures before/after syscall, panic between journal and commit, exact cleanup scope, background release/probe quarantine, bounded retries, supervisor unwind, join/retirement, resume and successor startup. Exercise real orchestration with controlled effects, not unreachable hand-constructed state alone.
- No allocation in healthy dispatch: existing rt_dispatch_no_alloc gate and representative paths (Up-only, Down-only, mixed, blocked repeat, wait/replan; observer off/on as supported). Retain memory-budget and large-plan scaling tests; report actual bytes and allocation counts with comparable baseline and workload, no invented savings. Maintain cancellation responsiveness.
- Wire/consumers: generated bridge and frontend consumers, schema 28/current fixtures plus historical immutable schema readers, evidence/qualification contracts and CLI harnesses. Preserve prior source-bound evidence; no rewriting history to make candidate pass. Run affected app checks and regenerate via existing tooling only where necessary.
- Dead-code audit: repository-wide search of old symbols and call sites (including scripts/xtask/tests/benches/examples/docs/feature flags/generated bindings); list removed files/symbols/LOC and remaining matches with concrete consumer or historical fixture justification. Check for unused private types, wrappers, reexports, config knobs, conditional paths and Cargo deps. No #[allow(dead_code)] / cfg tricks to mask dead paths; no blanket lint suppression. Test-only support must have a real test caller. Do not delete independent safety oracles or coverage just because private names changed.

## 6. Commands and evidence protocol

Use repository pins and Windows MSVC. Record exact commands, exit status, test counts, ignored/skipped cases, toolchain/OS, source SHA/tree, lock hash where relevant. Run focused checks during iteration; no pointless reruns of unchanged broad gates. At A checkpoint use affected crate tests, rustfmt and clippy per current workspace policy; include:
    node scripts/toolchain-pin.mjs exec -- cargo xtask check rust
    node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc -- --test-threads=1
    node scripts/toolchain-pin.mjs exec -- cargo check --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance
    node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- --test-threads=1
    node scripts/toolchain-pin.mjs exec -- cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- -D warnings
    pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
    git diff --check
Run security/static and affected Bun checks per owning runbook and actual changed paths. If exact command has changed in repository, use owning contract and report. At final relevant HEAD run:
    node scripts/toolchain-pin.mjs exec -- cargo xtask check all
Verify required aggregate CI at exact PR HEAD only for merge readiness, not as a prerequisite to review. A skipped or unrun gate is NOT PASS. Do not use --skip-supply-chain as a substitute. If environment missing, report blocker and actual covered checks. No disabling CI/gates, changing tolerances, weakening assertions, broad allowlists or skip tests to get green.

Physical input: NOT AUTHORIZED by this work order. Do not send real keys or use -OwnerApprovedReceiveOnlyRun without the owner's explicit machine/receiver/scope authorization per SECURITY.md and playback-verification.md. Use mocks, dry-run/PlanOnly and nonphysical tests now. Preserve logs outside tracked source, secrets out of logs; provide raw evidence access via authorized artifacts or concise redacted excerpts. Don't upload large raw traces as product files.

## 7. Agent coordination / review

One draft PR, one branch. Leader will review exact pushed SHA and cumulative baseline diff + package delta, code reachability, tests, dead-code audit, regressions, CI signals when available. No automatic review or background monitoring is implied; owner invokes leader in chat with SHA/PR. Leader records findings and next work order in same PR. CLI may choose private names/algorithms within frozen contract and fix implementation bugs; must escalate material architecture, policy, external/wire changes, security, test relaxation, dependency, timing defaults, benchmark thresholds, ambiguous compatibility, or unverified claim. No self-approval. For blocker, supply minimal reproduction, actual source paths, evidence, options/tradeoff and recommendation; continue independent authorized tasks where safe.

PR checkpoint comment template (CLI):
    Package: A / B / C
    HEAD / tree / base / clean worktree:
    Summary of end-to-end changes:
    D1-D9 mapping / any deviations (none or explicit blocker):
    Old code removal + remaining consumer audit:
    Tests/commands + exact outcomes/skips:
    Behavioral equivalence/independent oracle evidence:
    Memory / performance data (or NOT MEASURED):
    Native physical/game evidence: NOT RUN unless explicitly authorized
    CI status: informational (review requested regardless)
    Residual risk / decisions requested:
    Ready for leader review: yes / blocked

Acceptance is not just compilation: leader must verify production has one canonical plan and dispatch path, no obsolete private implementation, behavior and safety contracts maintained, tests credible and evidence bound to source. Final merge requires owner/leader decision and canonical repository gates. Do not merge, release or deploy on your own.

# Source architecture consolidation — implementation and leader control plan

**Revision:** 1.0 | **Status:** active work order, WP1 authorized only
**Integration branch:** `refactor/source-architecture` → `main`
**Coordination:** [Issue #689](https://github.com/pumni/Sky-Auto-Player/issues/689)
**Design:** [SPEC v0.2](source-architecture-spec.md), including D01–D20 and R01–R15.
**Research/base commit:** `5c79c8cf604072f331820126223188161d17d7b1`.
The owner's instruction to proceed authorizes this program and the WP1 baseline. The leader selects root workspace as the preferred design, conditional on feasibility evidence. WP2 and later are planned, NOT dispatched. This plan records execution authority, not a second permanent architecture contract. AGENTS.md, CONTRIBUTING.md, SECURITY.md and the owning documents in INDEX.md remain binding.

## 1. Operating model

| Role | Authority |
|---|---|
| Owner | Scope/strategy approval, final merge decision, release/credentials/physical-input authorization |
| Leader (ChatGPT) | Decide layout, architecture, module cuts, scope, contract changes, risk tradeoffs, review findings, accept/reject each SHA and dispatch next work package |
| Implementation agent (CLI) | Inspect actual checkout; implement explicitly authorized outcome; choose routine implementation details inside approved boundaries; tests, format, lint, regression evidence, cleanup, commit and push; report blockers |

No agent self-approval. Material decisions include new crates/dependencies/features, package names, public/wire contract changes, synchronization or state ownership, test oracle/skip/tolerance changes, compiler/profile/default member/target policy, safety/release/CI authority changes, exception additions, or unexpected scope growth. Escalate with concrete evidence, options, recommendation and affected invariant; leader decides in #689 before implementation. Local routine fixes within the authorized boundary require no ceremony. Never weaken a gate, replace a real oracle with a mock, or silently expand a check exception.

### Single-branch protocol

1. Fetch origin; inspect status, branch and current remote SHA. Switch to the authorized branch tracking origin. Never overwrite local work. Never create another implementation branch/PR. Read the latest leader instruction in #689 plus this plan and SPEC at branch HEAD.
2. Work in substantial, cohesive packages below, with reviewable internal commits (conventional subjects). A package is an outcome, not a quota of files; finish all affected callers, checks, docs and removal in the same slice. No artificial micro-handoffs for individual files.
3. Use fast-forward-only synchronization with the integration branch. No force-push/rebase rewriting reviewed history; no merge from main, dependency PRs or other branches without leader direction. If main advances or conflicting changes appear, report both SHAs and ask leader to authorize integration/revalidation. Do not start release while moving topology.
4. Test the actual committed source or explicitly identify dirty input; push the exact reviewed candidate, not a different tree. Record source/tree SHA and remote HEAD after push. Use the one cumulative draft PR for diff/review, #689 for dispatch and status. Review each accepted-checkpoint-to-current-HEAD diff AND cumulative base-to-HEAD risk.
5. **Review now, CI later:** after local work and evidence, push and post handoff immediately; no polling/wait for hosted CI. If a required local run is still executing, distinguish progress from complete handoff and attach result when available; do not report an aborted run as pass. Leader may request corrections or accept source provisionally while hosted checks are pending; may authorize next nonconflicting package explicitly. Later failure invalidates affected evidence and blocks final acceptance until resolved. Never confuse review acceptance with merge readiness.
6. Stop at handoff until explicit next dispatch. No self-assigned follow-on or automatic merge, release, metadata promotion, signing with production keys, real/ReceiveOnly input qualification or deployment. Owner must authorize the applicable machine, receiver and scope for physical input; follow playback-verification.md. No secrets, keys, installer binaries, huge logs, machine-specific paths or private data in Git.

## 2. Fixed decisions and non-goals

- One virtual root Cargo workspace; six libraries under root crates/; root xtask/ and build_support/; existing desktop src-tauri stays under apps/desktop. Preserve eight members and all package, binary, product and installer identities. Independent desktop/site Bun locks remain.
- Move the exact existing Cargo.lock and rust-toolchain.toml. Preserve resolver, current pins/MSRV, target, engineering release and shipping dist profile; no upgrade or performance flags. No new default-members, new crate or monorepo tooling. Keep explicit member list.
- Local Cargo discovery becomes conventional; trusted CI/release must verify compiler and overrides before any compilation, preserve containment, lock mode, fixture environment and exact artifact handoff. Retain useful verification; remove obsolete nested-workspace routing only when proven replaced.
- Structural extraction first. Preserve state fields/locks/lifetime/order, algorithms, persistence and IPC. No runtime ownership redesign, dispatch hot-path rewrite, optimization, UI redesign, resource relocation, blanket crate renaming or historical-layout framework.
- Do not duplicate production code or maintain old/current implementations in parallel. Stable public/wire/external IDs, necessary data migrations, public re-exports and immutable historical records are NOT dead code. Prove why they still exist.
- Guidance: normal implementation approximately 500 LOC, review at 800 excluding identifiable test/generated regions; no split quota. Keep existing safety hard limits/allowlist semantics. No generalized LOC analyzer, metrics database, automatic API counter or new governance framework.

## 3. Milestones and dependencies

| Package | Coherent scope | Exit / leader decision |
|---|---|---|
| WP1 (active) | Full repository topology audit + practical exact-base baseline | Approve/reject root migration feasibility and concrete path map |
| WP2 | Atomic source/workspace cutover plus all active build/CI/release/test/doc consumers | Accept topology only after local evidence; track pending required hosted proofs |
| WP3 | Workspace dependency/metadata/lint inheritance and audit coverage | Approve equivalent effective configuration; no new feature edges |
| WP4 | Core/adapters and desktop IPC/runtime organization, internally separate coherent commits | Accept physical extractions after focused review and relevant evidence |
| WP5 (conditional) | Focused higher-risk playback/updater/xtask/frontend consolidation | Leader selects exact slices after focused audit; defer with reasons otherwise |
| WP6 | Final cleanup, full canonical validation, exact candidate qualification and owning docs | Owner acceptance, merge readiness only after required local and hosted gates |

WP2 and WP3 deliberately separated so dependency drift is attributable. WP4 can have two internal review checkpoints (core/adapters; desktop shell) only if risk warrants, not a per-file sequence. WP5 may be omitted in whole or part. Existing checks and contracts—not this table alone—determine necessary lanes. Final source must be revalidated after material changes.

## 4. WP1 — FOUNDATION AND BASELINE (execute now)

**Outcome:** a reproducible baseline and complete migration-consumer map sufficient for the leader to decide and authorize WP2 without guessing. No source migration or product code edits. This is repository-wide work, not a few-file assignment.

### 4.1 Source and tooling baseline

- Confirm base/main, current branch HEAD, tree, worktree cleanliness and latest #689 authority. Planning commits are docs-only; distinguish research SHA from actual tested HEAD. Record current remote main and overlapping open PRs without merging them. Inspect actual Git-tracked files, not generated build directories or node_modules.
- Record OS, architecture, Rustup active toolchain, rustc -Vv, Cargo version, Bun version, relevant MSVC/SDK/NSIS/WebView2 versions where available; note environment overrides (names and safe values only), lockfile hash, .bun-version, workspace manifest/profile and toolchain pin. Use the current pinned wrapper from AGENTS.md; no installation/upgrade of different toolchain. Record workspace package identities/targets, normal/build/dev/target dependencies and feature declarations, exact paths and current Cargo default behavior.
- Capture locked metadata and target-specific dependency/feature trees, separately for production and qualification with actual feature switches taken from canonical builders; do not mistake all-features graph for shipping graph. Record full commands and preserve sanitized raw evidence outside Git if large. Compare later by normalizing only first-party path/package IDs, not version/source/features.
- Example from repo root (PowerShell; check actual supported flags/contexts against source):
  ~~~powershell
  git rev-parse HEAD
  git rev-parse 'HEAD^{tree}'
  git status --short
  node scripts/toolchain-pin.mjs read rust
  node scripts/toolchain-pin.mjs verify rust
  node scripts/toolchain-pin.mjs exec -- cargo metadata --locked --manifest-path rust/Cargo.toml --format-version 1
  node scripts/toolchain-pin.mjs exec -- cargo tree --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell --target x86_64-pc-windows-msvc -e normal,build
  node scripts/toolchain-pin.mjs exec -- cargo tree --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell --target x86_64-pc-windows-msvc -e features
  ~~~
  These are starting commands, not a substitute for the actual Tauri build feature/profile invocation. Check locked mode at the actual Cargo subprocess; do not confuse xtask --locked with shipping build --locked. If missing, document pre-existing defect with source/log proof and propose separate reviewed fix.

### 4.2 Comprehensive path inventory and cutover map

Inventory every *current* consumer using tracked-file search (git grep, git ls-files, IDE/rg as needed) and semantic inspection. Include slash/backslash paths, relative traversals, include!/include_str!/include_bytes!, #[path], CARGO_MANIFEST_DIR parent counts, git root detection, manifest/lockfile detection, working directory, target directories, cache keys, path filters, glob patterns, generated contract inputs, embedded test fixture strings and exact test selectors. Literal search alone insufficient. E.g.:
~~~powershell
git ls-files rust
git grep -n -I -F -e 'rust/' -e 'rust\' -e 'rust-toolchain.toml' -e 'Cargo.lock' -e 'CARGO_MANIFEST_DIR' -e 'CARGO_TARGET_DIR' -e '--manifest-path' -e 'target/dist' -e 'target/release' -- .
~~~
Categorize each active occurrence or related cluster: path:line, owner/function, current/new path, required test, risk, whether immutable history. Identify nonliteral paths and tracked files overlooked by the query. Preserve genuine historical links and test data with documented reason; distinguish fixture strings that are active expectations from immutable historical provenance. Do NOT blindly replace.

Required coverage:
| Area | Specific inspection |
|---|---|
| Workspace/Cargo | members, external shell workspace, internal path deps, .cargo aliases/config, root discovery, lock and profile, target ignore |
| Pin/verification | scripts/toolchain-pin.mjs and tests, setup action, Rust checks/rust_toolchain.rs, override detection, cache keys |
| xtask | repo::root parent assumption; checks, audits, binding generation, CI impact, build support, subprocess CWD, path allowlists, failure handling |
| Tauri | src-tauri Cargo manifest, additionalWatchFolders, build.rs git metadata #[path]/rerun, resource paths, permissions/config, frontendDist, production invocation |
| CI/control | ci.yml, composite actions, classifier (new root paths, renames/deletes, full dispatch, unknown paths), required aggregate name, artifact names/exact SHA, caches |
| Release | release_v4 builder, target/dist, xtask.exe and output discovery, release contracts, supply chain (audit/vet/SBOM), signing/attestation/recovery; exact bytes and permission boundaries |
| Fixture | updater_fixture.rs same-current-source synthetic bridge, validated worktree and lockfile rewrite; candidate and bridge isolated CARGO_TARGET_DIR; no historical layout autodetection |
| Source/test | includes and relative paths, exact test filters, fixtures, test-only feature gates, Rust bindings and TypeScript generated output; retained public API |
| Repo docs/scripts | AGENTS, CONTRIBUTING, owning docs, .gitignore, .github automation/dependency updates, scripts and tests; historical provenance preserved |

Read actual source at HEAD and record risks with file/function citations. Identify any path that must intentionally remain and why. Draft the atomic move list and test plan, including negative path/classifier/override checks; leader will approve it rather than CLI choosing fallback topology.

### 4.3 Validation and measurement baseline

- Check existing exact-source hosted CI/required status and run links; distinguish base from docs-only branch HEAD. Do not wait on a run. Existing evidence may be reused ONLY as allowed by owning policy, never silently transferred to different source. Run relevant local focused checks and the canonical pinned `cargo xtask check all` once on the baseline if no adequate exact-source evidence and environment allows. Avoid redundantly running broad subgroups then aggregate. Record raw commands/exit statuses/durations, discovered/executed test counts and failures/limits. No physical-input acceptance, game process or production credentials.
- For future comparability capture readily available baseline build/installer/executable sizes and build wall time from an existing canonical nonproduction candidate or safe authorized local packaging route. Record exact SHA, profile/target, toolchain, machine, cache conditions, artifact hashes, commands and sample count; if unavailable mark not measured. No new benchmark harness, no optimization or absolute performance threshold. Do not run a production release transaction or publish. Any special Windows package proof uses disposable paths and existing authorization rules; report missing authorization as blocker. A browser/mock success is not a native proof.
- Determine whether full baseline validation, feature equivalence or exact package verification is missing; distinguish existing failure from introduced failure. Report repro for baseline defects; do not fix them silently in WP1. Review cost/risks of root layout versus pin-only option. Leader decides.

### 4.4 Deliverable and review gate

Create `docs/evidence/source-architecture/WP1-baseline.md` (or one linked Markdown table/report within that directory); include source identity, graph/pin/profile, inventory with dispositions, test/build evidence, exact migration map, risks and explicit questions. No large generated graph blobs, binary artifacts, secrets or unverified claims. Keep historical evidence revision-bound. If large raw logs needed, store externally with accessible links or sanitized excerpts; state what is unavailable. Commit evidence; push; post WP1 READY FOR LEADER REVIEW or WP1 BLOCKED in #689. Stop. Leader reviews feasibility and issues WP2 dispatch. A doc-only commit after testing must identify the actual tested source and changed files; do not claim exact-HEAD qualification without contract-backed evidence.

## 5. WP2 — ROOT WORKSPACE CUTOVER (planned; await dispatch)

### Scope and implementation

Move all six library directories, xtask, build_support, root manifest/lock/pin in one coherent cutover. No changes to library logic, public APIs, resolver/default members, dependencies, features, profile values or package names. Use git mv or equivalent clean tracked renames. Do not copy old target output. Update Tauri membership and path deps without centralizing them yet; keep shell location. Set root xtask alias to run --locked --package sky_xtask --bin xtask --. Replace two-parent repo root derivation with validated actual root independent of CWD. Use simple fixed target path when contract fixes it; retain isolated/overridden output handling and containment. Remove obsolete nested workspace routing; keep pin verification/setup action and reject unauthorized compiler overrides before compiling trusted xtask or product. Update every WP1 active consumer, including toolchain tests that reject the new root pin, CI cache and path classifier rules, release candidate and SBOM/vet/audit paths, Tauri watch/build support, binding and fixture paths. Synthetic bridge worktree uses root Cargo.lock from same source SHA; do not build fallback dual layouts. Update live docs and ignore rules; preserve immutable history. No separate compatibility workspace, shadow alias, duplicated lock, orphan script or stale CI exception.

### Required evidence

- Root discovery from root, crates, desktop shell and xtask; locked metadata and before/after normalized member/dependency/feature comparison, lock hash/diff and unchanged profiles. Local ordinary Cargo and trusted verification tested, including negative overrides/conflicting pins.
- Regression tests for classifier new root files, crates, xtask, build_support, renames/deletions, union, unknown/full dispatch; preserve aggregate name and proof coverage. No test-selector loss.
- Canonical checks (format, lint, production-only and all-feature tests per existing policy), generated bindings drift check, Tauri dist invocation/lock/feature verification, exact NSIS package and synthetic updater qualification under existing nonproduction authority. Report unavailable host-only proofs; they remain pending, not waived.
- Removal ledger: tracked rust/ gone, one live workspace/pin/lock, no active old target consumers; grep old path and classify every intentional residual (history, external stable identity, fixture with explicit reason). All removed helper callers updated and old tests either migrated or demonstrably obsolete. No unreferenced replacement shims or lint suppressions. Run focused tests then canonical gate; no repeat broad gates unnecessarily.
- Handoff with checkpoint diff, command outcomes, artifact/evidence links, CI pending status and rollback boundary. Leader may review immediately, accept code provisionally; cannot claim merge-ready.

## 6. WP3 — MANIFEST CONSOLIDATION (planned; await dispatch)

Centralize first-party paths and compatible shared dependencies/metadata/lint inheritance only. Keep consumer-specific default-features, features, optionality, aliases, kinds, target predicates, versions/source and exceptions. Keep exact resolved lock graph; no automatic cargo update. No new lint policy or broad suppression. Fix existing architecture dependency checks so inheritance/aliases and target/kind variants cannot bypass current forbidden edges (including Windows-only adapter); add positive and negative regression fixtures exercising actual effective declarations. No generic policy engine.
Compare declared and normalized locked graphs and feature trees on equivalent production/qualification modes, actual canonical production invocation and available compiler artifact evidence. Verify test-only and qualification-only feature isolation, current hard safety checks, lint/format/test and applicable/full canonical lanes. Remove old direct redundant declarations once migrated, not alternate config. Handoff for separate leader acceptance.

## 7. WP4 — SOURCE ORGANIZATION (planned; await dispatch)

Before each coherent domain batch, inspect source owners/callers, tests and exact selectors. Leader approves concrete module extraction map when material. Suggested batches:
- **Core and native adapters:** sky_app_core catalog identity/index/ranking/normalization and song decode/planning/schedule/risk; sky_native_adapters lib.rs storage, catalog composition/sources and atomic filesystem helpers. Extract existing coherent implementation into descriptive private modules and deliberate facade exports; retain algorithms, public paths and atomic I/O semantics. Move substantial related unit suites without losing tests, preserve fixtures and test selection. No broad visibility widening, new crates or duplicate APIs.
- **Desktop delivery/runtime:** src-tauri lib.rs composition and tests; commands.rs DTOs by capability; native_runtime/catalog/library and bootstrap/settings/lifecycle implementation ownership. Keep existing runtime structs, mutation authority, Arc/Mutex/atomic/Condvar and lock acquisition/order, cancellation and shutdown unchanged. Preserve serialization, generated bindings, IPC names, events, persistence, lifecycle, window behavior. Review exact caller paths, command permissions, binding export ordering and relevant integration/packaged tests.
Within the authorized scope do whole responsibility migrations, not isolated file renames. Use sibling test files for substantial suites, keep small cohesive inline tests. Source triage is advisory (500/800); do not force arbitrary splits or create empty modules. For each extraction record old symbol owner -> new owner, migrated callers/tests and deleted old implementation. Compare relevant test discovery names/counts, run exact filters and verify nonzero intended tests, no silently weakened oracle. Preserve stable API paths with intentional facade re-exports, never retain private obsolete implementations as fallbacks. Run owning native/frontend and applicable package lanes. Review local evidence and diff, not LOC alone.

## 8. WP5 — TARGETED CONSOLIDATION (conditional; await dispatch)

Leader chooses only justified slices following focused audit; agent provides findings but does not choose scope. Candidates: desktop playback preparation/start/monitor/control boundaries (session.rs), native updater state/metadata/progress, xtask release_v4 transaction stages, updater fixture server/harness/evidence, CI classifier rules/tests, and frontend state capability grouping. Preserve existing transaction order, authority, artifact bytes, trust, cleanup, runtime state, session identity and event ordering. Do not create new ownership/synchronization architecture. Keep one Zustand store and typed bridge. Engine hot path and native session redesign out of scope. For sensitive slices provide precise invariant/caller/lock or transaction map and proof strategy before extraction; use existing negative/regression/contract tests and authorized package/native checks. No broad release rewrites. Record justified deferrals; no new abstraction just to reduce LOC.

## 9. WP6 — FINAL ACCEPTANCE (planned; await dispatch)

Reconcile all accepted checkpoints and residual issues; inspect cumulative diff and entire changed subtree. Run canonical final validation on final relevant source, exact Windows NSIS candidate install/launch/uninstall/resource and updater proof, required hosted aggregate at exact PR HEAD, applicable release security/authority proofs. Compare normalized lock/feature/profile and original IDs; verify no dormant old source paths or implementations. No production signing/publishing; final qualification uses existing approved nonproduction route. Optional performance evidence is descriptive: same hardware, OS/toolchain/target/profile/cache/workload, sample count, raw results, startup/size/build time if relevant; no performance claims without controlled comparable data. Native timing/physical qualification only with owner approval.
Integrate lasting rules into owning AGENTS/CONTRIBUTING/architecture/toolchain/CI/build docs and docs/INDEX as needed; archive this temporary plan and evidence according to HISTORY/evidence convention, avoid duplicate live authority. Record final acceptance gate matrix (SPEC §10.1), unresolved/pending failures and required owner decision. No merge before final exact-HEAD required gates and owner authorization.

## 10. Definition of done — mandatory cleanup for every implementation package

1. No old and new implementations coexisting without explicit approved public/compatibility obligation. Delete replaced private modules, functions, types, adapters, files, exports, imports, tests only of retired mechanisms, scripts/config entries, feature flags only if truly obsolete and approved, comments/docs and obsolete path allowlists. Preserve live migration semantics, data, security oracles and historical provenance. No TODO placeholder or stashed follow-up to finish the migration.
2. Search removed paths and symbols across tracked repo, including tests/fixtures/workflows/docs; inspect results semantically. Check new modules are reachable, unused imports/dead code warnings, actual callers and generated output. Existing lint gates must remain unchanged in strength; no new allow(dead_code), allow(unused), blanket public visibility, dummy use, ignored tests, relaxed thresholds or added compatibility shim to hide unfinished work. Escalate genuine exceptions.
3. Attach removal ledger: old location/symbol, replacement or reason removed, migrated consumers, negative search evidence and retained intentional references. Confirm no stale generated files or untracked intended files. Use git diff --check, git status, review git diff --name-status --find-renames and scoped search; inspect cumulative diff. Empty folder/duplicate lock/build output must not be committed.
4. Behavioral and evidence integrity: preserve tests and independent oracles; reconcile moved names/filters. Report every check pass/fail/skipped/not run, why, and exact source identity. Fix introduced failures before handing off or mark blocked. Do not claim zero dead code solely because lint passed.

## 11. Handoff format (post to #689, link from draft PR when useful)

~~~text
WP<n> READY FOR LEADER REVIEW | WP<n> BLOCKED
Accepted base SHA:
Pushed HEAD SHA / tested tree SHA / cleanliness:
Scope completed / commits:
Evidence file(s) / raw log links:
Commands (CWD, flags, exit code, test count, duration):
Package/fixture/artifact identity and hashes if applicable:
Cleanup ledger / old paths and symbols / intentional retained exceptions:
CI status at exact HEAD (pending / pass / failed / not run; run links):
Residual risks / decisions needed / known baseline failures:
No unauthorized architecture, dependency, behavior, safety or release changes:
~~~

Leader reviews diff, targeted implementation, tests, source binding and cleanup. Verdict: CHANGES REQUESTED, BLOCKED, SOURCE ACCEPTED (CI pending) or ACCEPTED (required evidence complete). Source acceptance is not merge approval. Leader posts explicit next dispatch only; CLI stops until then. Corrections stay on same branch. Owner alone authorizes merge/release. No automatic/background review is assumed: owner returns with handoff URL/SHA for leader to inspect GitHub.

## 12. Technical references and current authority

- [Cargo workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html), [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html), [dependency inheritance](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html), [Rustup overrides](https://rust-lang.github.io/rustup/overrides.html).
- [GitHub review workflow](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/reviewing-proposed-changes-in-a-pull-request); exact source/required gate comes from repository policy, not GitHub review status alone.
- [CI contract](ci-validation-topology.md), [Rust pin](rust-toolchain-policy.md), [Packaging](v4-tauri-packaging.md), [Release authority](v4-release-authority.md), [Playback verification](playback-verification.md), [Architecture](architecture.md), [Contributing](../CONTRIBUTING.md), [Security](../SECURITY.md).
- [Baseline updater fixture](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/rust/xtask/src/updater_fixture.rs#L2028-L2034) requires current HEAD match; [lockfile rewrite](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/rust/xtask/src/updater_fixture.rs#L2205-L2218) is current-source fixture, not historical source.

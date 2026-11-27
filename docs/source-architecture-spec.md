# Sky Auto Player — Source Architecture Specification

**Revision:** 0.2 — revised proposal for owner approval  
**Date:** 2026-11-28  
**Research baseline:** [`5c79c8cf604072f331820126223188161d17d7b1`](https://github.com/pumni/Sky-Auto-Player/tree/5c79c8cf604072f331820126223188161d17d7b1)  
**Supersedes:** draft revision 0.1, subject to owner approval  
**Suggested planning location:** `docs/source-architecture-spec.md` (owner to confirm)  
**Status:** Design only. Does not authorize implementation, merge, physical-input qualification, production signing, publication or metadata promotion.

## 1. Purpose and design principles

Establish a conventional Rust-first application monorepo that is easy for maintainers and coding agents to navigate, change and validate. Improve source discoverability, cohesion, encapsulation and change locality before starting performance engineering. Preserve observable behavior and existing safety, build, persistence, IPC, updater and release contracts.

This is targeted consolidation, not an architecture rewrite. Fewer directories, more files, lower LOC and new abstractions are not success metrics by themselves. A change is justified when responsibility becomes clearer or maintenance cost decreases without weakening correctness.

Normative language: MUST indicates an acceptance requirement, SHOULD a preferred default with contextual exceptions, MAY an optional choice. These requirements become effective after owner approval. Existing owning contracts remain authoritative until amended through the repository's normal review process. A conflict is a blocker to resolve, not permission to silently weaken a contract.

## 2. Evidence and scope

The research snapshot contains 286 tracked Rust files totaling 6,383,017 bytes, including tests and tools, across eight workspace members (six libraries, desktop shell and xtask). The eight Cargo manifests and selected implementation, CI, release and audit files were inspected. Main was rechecked at the above SHA on 2026-11-28. These observations are not production LOC, resolved compilation units or qualification results. Rebaseline the actual implementation base before changing code.

In scope: root workspace and toolchain relocation, current-source path consumers, compatible manifest inheritance, source/module conventions, selected physical extractions, tests, documentation and preservation of existing architecture enforcement. Targeted higher-risk extraction is conditional on focused evidence and owner-reviewed scope.

Out of scope: new features, UI redesign, blanket renaming, new crates by default, resource/catalog relocation, dependency/compiler upgrades, changed default build selection, profile optimization, IPC/persistence/schema changes, state ownership or concurrency redesign, dispatch/timing changes, release trust or authority changes, a historical-layout compatibility framework, and new governance frameworks. Existing product version, package and installer identities, user data locations, required check names and release contracts remain unchanged.

### 2.1 Change categories

- **Structural:** move files, adjust imports, preserve APIs and behavior; default scope.
- **Encapsulation:** narrow visibility or improve the internal API after identifying actual callers; separate reviewable slice when needed. Do not hide DTO fields mechanically.
- **Architectural redesign:** change ownership, dependencies, synchronization, lifetimes or state machines; requires a separate proposal and approval, not implicit authority from this SPEC.
- **Optimization:** change algorithms, allocation, scheduling or compiler settings; belongs to a subsequent measured performance program.

## 3. Architecture decisions

The identifiers D01–D20 are retained for continuity with revision 0.1.

| ID | Decision | Rationale / acceptance signal |
|---|---|---|
| D01 | Root virtual Cargo workspace | Conventional discovery and one workspace build boundary |
| D02 | Retire active `rust/` topology | No stale current-source path consumers; preserve historical records |
| D03 | Root `crates/` for six libraries | Role-based source organization |
| D04 | Retain `apps/desktop/src-tauri/` | Application owns delivery, IPC, capabilities and packaging |
| D05 | Root `xtask/` and `build_support/` | Separate engineering and shared build-time roles |
| D06 | Preserve eight member/package identities and target roles | Avoid unnecessary crate or naming churn |
| D07 | Inherit only compatible metadata, dependencies and lints | Preserve versions, dependency kinds, target predicates and effective features |
| D08 | Prefer thin entry points and facades | Composition and deliberate exports, not unrelated implementations |
| D09 | Narrow visibility according to invariants and callers | No blanket visibility widening or gratuitous getters |
| D10 | Cohesive capability/invariant modules | No generic architecture layers introduced for appearance |
| D11 | Normal implementation guidance: 500 target / 800 review | Review aid, not universal hard limit or refactor quota |
| D12 | Prefer descriptive sibling tests for substantial new suites | Keep cohesive small inline tests; preserve discovery and selectors |
| D13 | Preserve existing hard safety/function limits | No weakening of constraints or exceptions through migration |
| D14 | Preserve runtime ownership; physical extraction first | No new service containers, locks or changed mutation authority by default |
| D15 | Retain canonical Zustand store and typed IPC | One authority for native state and no wire drift |
| D16 | Reuse existing xtask checks; keep governance minimal | Machine-enforce objective contracts, review qualitative conventions |
| D17 | Freeze compiler and dependency resolution | Isolate structural changes; separately review any baseline defect |
| D18 | Preserve engineering and distribution profiles | Production must still use `dist`; no unmeasured tuning |
| D19 | Preserve product, persistence, package, update and release semantics | Canonical applicable qualification on the exact candidate |
| D20 | Separate source architecture from performance engineering | No performance claims from structural changes |

### 3.1 Alternatives considered

| Option | Benefit | Trade-off and decision |
|---|---|---|
| Keep existing nested workspace | No migration risk | Retains custom root discovery and sibling shell relationship; not selected for long-term layout |
| Move only toolchain pin to root | Solves normal Rustup discovery with smaller change | Does not solve Cargo discovery, external shell membership or nested manifest paths; viable fallback if full migration cost is disproportional |
| Rename `rust/` to `sky-rs/` | Clear subsystem branding | Retains nested-workspace issues; not selected |
| Root virtual workspace and root `crates/` | Aligns Cargo, repository and toolchain boundaries | Requires coordinated migration of tooling and package consumers; selected, subject to Phase 1 feasibility evidence |
| Move Tauri shell into `crates/` | All Rust packages in one directory | Weakens product-local delivery ownership; not selected |

A root workspace is a project-specific design choice, not a universal Rust requirement. Cargo supports nested workspaces and Tauri supports a shell as a workspace member. Do not add Bazel, a root JS workspace, new package orchestration or new architectural layers merely to imitate another project.

## 4. Target topology and build contract

```text
Sky-Auto-Player/
├── .cargo/config.toml
├── .github/
├── apps/
│   ├── desktop/
│   │   ├── src/
│   │   ├── src-tauri/
│   │   ├── package.json
│   │   └── bun.lock
│   └── website/
├── crates/
│   ├── sky_app_core/
│   ├── sky_native_adapters/
│   ├── sky_player/
│   ├── sky_dispatch_core/
│   ├── sky_dispatch_win32/
│   └── sky_windows_security/
├── xtask/
├── build_support/
├── scripts/
├── docs/
├── songs/
├── builtin-songs/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── .bun-version
├── AGENTS.md
├── CONTRIBUTING.md
└── SECURITY.md
```

Selective tree: retain other legitimate root files/directories. Desktop and website keep their independent Bun manifests and locks. Do not create empty directories, compatibility stubs, another live workspace or another lockfile. The Tauri shell remains a member under its application directory.

| Current source | Destination |
|---|---|
| `rust/Cargo.toml` | `Cargo.toml` |
| `rust/Cargo.lock` | `Cargo.lock` |
| `rust/rust-toolchain.toml` | `rust-toolchain.toml` |
| `rust/crates/` | `crates/` |
| `rust/xtask/` | `xtask/` |
| `rust/build_support/` | `build_support/` |
| `rust/target/` (untracked default output) | `target/` (new default, not a tracked move) |
| `apps/desktop/src-tauri/` | unchanged |

### 4.1 Workspace manifest

The topology slice moves the manifest without changing member names, target roles, resolver, metadata semantics or profiles. Preserve the values at the accepted implementation base; the following shows the research baseline, not a replacement for the complete live manifest.

```toml
[workspace]
resolver = "3"
members = [
    "apps/desktop/src-tauri",
    "crates/sky_app_core",
    "crates/sky_native_adapters",
    "crates/sky_player",
    "crates/sky_dispatch_core",
    "crates/sky_dispatch_win32",
    "crates/sky_windows_security",
    "xtask",
]

[workspace.package]
edition = "2024"
rust-version = "1.99"
authors = ["Sky Auto Player Team"]

[profile.release]
lto = false
codegen-units = 16
opt-level = 2
panic = "unwind"
strip = "symbols"
overflow-checks = true

[profile.dist]
inherits = "release"
debug = false
debug-assertions = false
lto = "thin"
codegen-units = 1
opt-level = 3
```

Keep explicit member list. No new `default-members`, publish/version policies, resolver changes or dependency updates. Adjust the shell's explicit workspace pointer or remove it only after confirming root discovery. Inherit dependencies and lints in a separate manifest slice (Section 5), not in the topology move.

### 4.2 Developer and trusted execution

Root `rust-toolchain.toml` is the single pin (baseline compiler 1.99.1; Windows target `x86_64-pc-windows-msvc`). Local developers may use ordinary Cargo commands from the workspace without a mandatory JavaScript routing wrapper. Rustup discovery follows ancestors, but environment, explicit and directory overrides can take precedence.

Trusted CI/release entry points MUST verify the pinned compiler and reject unauthorized overrides before compiling xtask, the product or qualification tools; apply the same policy to spawned commands and build scripts. Keep the existing resolver and setup action where they still perform verification and installation. Remove only obsolete routing behavior after equivalent assurance is proven. Update positive and negative tests that currently reject the root pin; preserve rejection of conflicting pins and unauthorized compiler overrides. Direct local commands are not trusted release evidence by themselves.

Target alias:

```toml
[alias]
xtask = "run --locked --package sky_xtask --bin xtask --"
```

After cutover, `cargo xtask check all` is the normal local aggregate entry point under the verified compiler; actual trusted invocation follows the owning CI/toolchain contract. Before cutover use the existing approved resolver. Preserve existing Bun pin and generated-binding environment semantics.

### 4.3 Output and production build

Use simple validated workspace-relative paths where output is contractually fixed. The normal default is `target/` under the root, including `target/dist/bundle/nsis/` and `target/release/xtask.exe` when the command/target settings match. Explicit target triples or isolated `CARGO_TARGET_DIR` may change these paths. Inspect Cargo metadata when needed, not as an obligatory new framework. Retain containment, worktree isolation, ownership sentinels, output cleanup and trusted environment checks. Do not migrate old build artifacts.

Preserve the Tauri CLI's actual distribution invocation (`--profile dist`), production features, NSIS installer identity, packaged resource inventory and updater policy. Confirm the final Cargo invocation is locked, not merely xtask's own build. If the accepted base already lacks required locked-mode assurance, record the baseline gap and obtain a separately reviewed correction; do not mask it as a path move or waive the owning build contract. Keep compiler profile settings unchanged; do not claim identical binary bytes across different source revisions. Exact-byte identity remains mandatory within each candidate handoff.

## 5. Dependencies and API boundaries

### 5.1 First-party declared normal dependencies

| Consumer | Providers at research baseline |
|---|---|
| `sky_app_core` | none |
| `sky_dispatch_core` | none |
| `sky_windows_security` | none |
| `sky_dispatch_win32` | `sky_dispatch_core` |
| `sky_native_adapters` | `sky_app_core`; `sky_dispatch_win32` only on Windows |
| `sky_player` | `sky_dispatch_core`, `sky_dispatch_win32` |
| `sky_desktop_shell` | `sky_app_core`, `sky_native_adapters`, `sky_player`, `sky_dispatch_win32` |
| `sky_xtask` | `sky_app_core`, `sky_windows_security` |

`sky_player` also uses app core and dispatch crates in dev-dependencies. Preserve normal/build/dev distinctions, target conditions and production/test isolation. No new first-party edges, cycles, production dependencies on xtask or delivery/platform effects in pure cores in this program. Preserve existing unsafe boundaries and `#![forbid(unsafe_code)]` in pure cores. This is a declared first-party graph, not proof of the resolved or compiled graph.

### 5.2 Manifest inheritance and evidence

Inherit first-party paths and genuinely identical external version/source policy using `[workspace.dependencies]`. Retain consumer-specific `features`, `default-features`, optionality, target predicates and dependency kinds. Workspace features are additive. Do not widen feature requirements simply to reduce repetition. For example, if only one consumer needs `serde`'s `derive`, keep the feature at that consumer rather than adding it to every inheriting declaration. This is illustrative; inspect actual requirements before changing manifests. Do not force inheritance for incompatible requirements. Inherit only common lints/metadata that preserve existing levels, priorities, exceptions and unsafe policy; do not introduce blanket suppressions.

For topology and manifest slices, capture and compare on equivalent targets and build modes:

| Evidence | Required interpretation |
|---|---|
| Member manifests and workspace declarations | Package identities, aliases, version/source requirements, feature flags, dependency kind and target predicates |
| `Cargo.lock` | Preserve bytes if possible; otherwise explain path-related metadata changes and prove unchanged resolved package versions, sources, checksums and relationships |
| Locked `cargo metadata` | Normalize relocated first-party paths/package IDs; compare meaningful graph and metadata changes |
| `cargo tree` | Compare selected normal/build edges and feature activation separately for production and qualification; not a complete compilation-unit/link map |
| Actual canonical production build | Verify selected target, profile, lock mode and features; inspect available compiler-artifact JSON/logs where feasible |

Keep `test-support`, `real-input-acceptance`, `tauri-test`, `tauri-update-fixture` and measurement-only features out of shipping builds unless an existing owning contract explicitly permits a particular feature. Separate test and production evidence; do not infer shipping features from all-features checks. No nightly compiler or new unit-graph tool required. Missing required evidence or unexplained drift blocks the affected slice; investigate before acceptance.

The existing dependency audit MUST continue to detect forbidden edges under inherited, renamed, normal/build/dev and target-specific declarations as appropriate to each rule. Use the effective manifest/metadata representation or a small extension of existing checks. Add positive/negative fixtures for changed cases, including inherited aliases and Windows-only adapter conditions. Preserve scope of all existing hard rules; do not create a general policy engine.

### 5.3 Visibility

Private implementation by default; use `pub(super)`, restricted `pub(in ...)`, `pub(crate)` and `pub` according to actual callers. Explicitly re-export intended cross-crate APIs; existing wildcard exports may remain until a justified cleanup. Preserve reachable public paths during physical extraction. Never widen visibility simply to silence a move-related compiler error. Wire DTOs and invariant-free value fields may be public; state-bearing objects and validated values should restrict mutations that could violate invariants. Do not add getters, setters, traits or compatibility layers without a concrete need. Review changed API against actual consumers, not a declaration-count quota.

## 6. Source organization standards

Organize modules by behavior and invariant. Crate roots and entry points declare modules, expose intended API, document invariants and compose the application; cohesive small implementation may remain. Prefer `feature.rs` with `feature/child.rs` for new module families; existing `mod.rs` is valid. Use descriptive snake_case Rust file names. Avoid generic dumping grounds (`utils`, `common`, `manager`) but do not rename legitimate coherent modules for appearance. Prefer private child modules over a new crate; new crates require a separate demonstrated dependency, reuse, platform or ownership boundary.

### 6.1 Size guidance

- Normal implementation SHOULD target approximately 500 lines excluding identifiable test-only/generated code. Around 800 lines, review cohesion and prefer new independent behavior in a suitable module instead of growing unrelated responsibilities. Explain significant growth or a decision to retain cohesive large code in the PR; no quota, forced split or blanket exception database.
- Facades SHOULD remain easy to scan; retain existing scoped 250-line advisory where applicable. Do not introduce a new universal facade/test/safety-critical LOC taxonomy.
- Preserve existing scoped architecture diagnostics and hard worker/dispatch function rules (baseline 350/180 lines with their existing counting method, scope and exceptions). These are not universal limits; do not alter their semantics by switching to the new review metric. Existing context-field advisories are not a reason to wrap fields or add locks.

Use tracked source paths, physical line counts and obvious module roles for initial triage. Exclude identifiable unit-test/generated regions only when reliable; mark mixed or uncertain files as such. Do not equate approximate counts with effective production code. Reuse existing diagnostics where sufficient. No new AST-based LOC parser, public API counting framework, scoring dashboard or generalized allowlist system by default.

### 6.2 Tests and fixtures

Keep tests near their owner. For substantial new unit suites, prefer a descriptive sibling `*_tests.rs` (or cohesive `tests/` module family) attached with `#[cfg(test)]` and, when needed, `#[path]`. Small cohesive inline tests are acceptable; do not move tests solely for convention. Keep integration tests, examples and benchmarks in conventional Cargo locations. Keep qualification-only tools and fixtures isolated from shipping code. Do not expose production internals solely for testing.

When moving tests, reconcile old/new fully qualified names, selectors, filters, `--exact`, ignored tests, target requirements and CI scripts. Confirm actual discovery and intended execution counts; zero matched tests is not passing evidence. Preserve assertions, scenarios, fixtures and oracles. Do not substitute browser/mock tests for native or package qualification.

## 7. Desktop ownership and targeted extraction

Keep existing state owners: `AppState` for Tauri-managed integration/lifecycle, `NativeDesktopRuntime` for composition and coordination, `NativePlaybackService` for preparation/session/retirement orchestration, native engine session for execution, `ActivityCoordinator` for shared exclusion/recovery authority, `UpdateService` for updater state and `NativeEventHub` for event delivery. Preserve authoritative settings, catalog, library, analysis and playback data; Zustand remains the single UI store and projection/interaction owner, not native execution authority.

Default extraction moves implementation among private modules while retaining existing structs, fields, mutexes, atomics, `Arc`s, lock order, cancellation, lifetimes and transaction boundaries. Document the relevant owner and invariants before each affected slice; a detailed writer/lock/event map is needed only where the change touches those behaviors. No creation of service objects or redistribution of mutable fields is implied by a folder name. Any ownership redesign requires a separate proposal and qualification plan outside default scope.

Preserve IPC commands, DTO serialization, generated TypeScript bindings, permissions and frontend bridge rules. Verify binding output and wire tests after moving DTOs. Retain single Zustand store, slice coordination and existing event/reconciliation authority. Group frontend files only when focused review establishes discoverability benefit. No second store or new frontend architecture framework.

### 7.1 Candidate map (not an exhaustive mandatory file plan)

Paths below describe baseline responsibilities, with relocated root prefixes understood after Phase 2. Physical line counts include tests/comments and are approximate.

| Group | Candidate / baseline size | Possible improvement and constraint |
|---|---|---|
| A — default priority | `sky_native_adapters/src/lib.rs` (~2,112) | Separate settings/library stores, catalog source/composition and atomic file logic; preserve persistence, path and platform semantics |
| A | `sky_app_core/src/catalog.rs` (~827), `song.rs` (~858) | Separate coherent ranking/identity/schedule/risk implementation; preserve domain contracts and existing exports |
| A | Desktop `commands.rs` (~1,514), `lib.rs` (~1,475) | Group wire DTOs and entry-point tests; preserve serialization, bindings, capabilities and command registration |
| A | `native_runtime.rs` (~1,834), `native_runtime/catalog.rs` (~1,606) | Extract cohesive bootstrap/catalog/library/settings/lifecycle methods; retain state and synchronization |
| B — focused audit required | Desktop `playback/session.rs` (~4,454) | Consider preparation, capacity/validation, startup, monitoring and control extraction; keep existing retirement/presentation/diagnostics boundaries and lifecycle invariants |
| B | `native_update.rs` (~1,735) | Separate metadata/state/progress/fixture responsibilities where safe; preserve trust and transitions |
| B | `xtask/release_v4.rs` (~8,133), `updater_fixture.rs` (~3,685) | Extract independently testable stages or infrastructure; reuse existing release modules; retain transaction order and exact artifacts |
| B | `xtask/ci_classifier.rs` (~2,589), checks and frontend state | Separate policies or capability files only where needed; preserve required lanes, store and test contracts |
| C — deferred | Engine `worker/dispatch_loop.rs` (~2,485), `engine/session.rs` (~1,905) | No default hot-path or native-session refactor; separate evidence, scope approval and qualification required |

Group A is the starting priority, not a requirement to split every listed file. Group B can proceed only after a short focused finding explains maintenance value, callers, risks, proposed boundaries and sufficient tests; owner may defer it without preventing final acceptance. Group C is outside this program's default scope. File names in any proposal are examples, not instructions to create every file. Stop when the maintenance objective is met.

## 8. Topology migration and path consumers

Before relocation, enumerate tracked path consumers with source search and targeted reading. Classify current-source references, actual historical references, documentation/history, runtime/user paths, generated outputs and ambiguous hits. Inspect process CWD, `include!`/`#[path]`, relative path depth, cache/output paths and config discovery, not just literal strings. No global replace of `rust/`.

| Surface | Required migration check |
|---|---|
| Cargo manifests, `.cargo/config.toml` | Members, aliases, path dependencies, shared lock, binding environment |
| Toolchain resolver, tests, setup action, `checks/rust_toolchain.rs` | Root pin, verification/override rejection, workflow validation and cache keys |
| `xtask/src/repo.rs` | Replace old two-parent assumption with validated repository root independent of caller CWD |
| Tauri config/build scripts | Watch folders, shared `git_metadata.rs` path, frontend assets, resources and native packaging |
| CI, composite actions, classifier | Current paths, artifact/cache keys, exact source identity, required status name and conservative lane selection |
| Release tooling and checks | Target output paths, build invocation, provenance, signing/qualification handoff and permission boundaries |
| Updater fixture | Current-source synthetic bridge lockfile and manifest paths, isolated target, cleanup and artifact evidence |
| Supply chain and architecture/security audits | Lockfile and source roots, audit/vet/SBOM paths, exact exceptions and existing hard rules |
| Generated bindings/tests | Output locations, module paths, fixture includes, exact test selectors and execution counts |
| Ignore files, scripts, live docs | Root target ignore, working directory and command conventions; preserve immutable history |

### 8.1 Corrected updater fixture contract (R03)

At the research baseline, `build_bridge_internal()` validates that `source_sha` equals the checked-out Git HEAD (lines 2028–2034), creates a detached worktree at that same SHA (2142–2153), and rewrites bridge-version manifest/lockfile inputs (2205–2218). A synthetic previous-version identity is not a historical source checkout or an actual published previous installer. Revision 0.1 incorrectly conflated these cases.

For the current-source bridge, update the lockfile path to the relocated root within the validated worktree. Preserve source binding, fixture version/configuration, test-only features/keys, isolated target, ownership sentinels, cleanup and exact-artifact checks. Validate the actual synthetic bridge-to-candidate qualification required by current contracts. Do not claim this proves migration from an actual published release. If an owning contract explicitly requires a previously published installer, qualify that real artifact separately; if an actual historical-source rebuild is required, use that revision's own contract and layout. Do not add legacy layout auto-discovery, fallback workspaces or a historical compatibility framework by default. Preserve old tags, release assets and historical evidence unchanged.

### 8.2 Cutover transaction

Keep the topology change reviewable and internally consistent: move source and all active consumers in one coherent slice; no accepted checkpoint with broken paths. Preserve existing CI aggregate status, explicit full dispatch, scheduled validation, union of required lanes and fail-closed handling of unknown/invalid paths. For topology/control-plane changes require full validation; add regression fixtures for new path families and renames/deletions so no required lane is lost. Run canonical Windows package/updater qualification on the exact candidate. Do not weaken existing checks to accommodate the new directory names. If migration feasibility is unacceptable, return to owner review of the alternatives in Section 3.1.

## 9. Delivery plan — five phases

Each phase/slice requires approved scope, actual accepted base, focused evidence, reviewable diff and applicable existing gates. Five phases are checkpoints, not five large PRs. Topology, inheritance and individual extractions are independently reviewable; no simultaneous performance changes. Phase 4 may be deferred in whole or part with recorded rationale. No implementation starts before owner approval.

| Phase | Required work | Exit criteria |
|---|---|---|
| 1 — Foundation & Baseline | Approve decisions; refresh SHA, member/manifest inventory, toolchain/profiles, current path consumers, relevant graph and canonical status; settle feasibility and conventions | Minimal baseline sufficient for topology migration; known gaps explicitly recorded; owner approves migration slice |
| 2 — Workspace Modernization | 2A: coherent root topology and all path consumers; 2B: compatible manifest inheritance and audit adjustments as separate slice | Root and member discovery, verified compiler, unchanged identities/resolution/profiles, actual production config, required full CI and exact package/updater evidence; inheritance separately verifies effective features and existing rules |
| 3 — Source Organization | Group A focused physical extractions, one responsibility at a time; retain behavior, APIs and state | Relevant tests/selectors and canonical CI lanes pass; review demonstrates improved discoverability/cohesion without unrelated changes |
| 4 — Targeted Consolidation | Only justified Group B slices after focused audit; no automatic engine or ownership redesign | Applicable native/update/release/desktop evidence complete for accepted changes; optional deferrals recorded and justified |
| 5 — Architecture Acceptance | Final full canonical validation, exact Windows candidate qualification, residual issue review and owning-doc updates | Section 10 gates satisfied on final source identity; owner accepts baseline before separate performance program |

### 9.1 Just-in-time analysis

Phase 1 is not a full runtime audit. Capture workspace/package identities, tracked inventory, current topology consumers, compiler/profile and build/dependency baseline, and existing canonical validation status. A fresh full qualification need not be duplicated solely to create a planning report if existing exact-source evidence is sufficient under owning contracts; record missing or stale evidence. Before the affected slice, capture detailed public API/callers, state writers, lock/ordering/cancellation invariants, test selectors and regression oracles as relevant. No full 286-file AST analysis or global lock graph prerequisite. Do not approve a change whose relevant baseline is missing or whose regression risk cannot be validated.

## 10. Verification and acceptance

Use existing repository contracts and classifier; no parallel qualification policy. Focused checks accelerate iteration but never replace required hosted or native/package lanes. Do not rerun unrelated checks just to satisfy a new checklist, nor reuse results across source identities unless existing policy explicitly permits it.

| Change | Minimum evidence in addition to applicable owning contracts |
|---|---|
| Topology / CI control plane | Full validation, path classifier positive/negative tests, exact package and updater qualification |
| Manifest inheritance | Declared/resolved/feature equivalence, actual production config and audit fixtures; full canonical validation where classifier requires it |
| Pure core / adapters | Existing unit/integration/fixture tests; applicable canonical lanes |
| IPC / native runtime extraction | Bindings and wire tests, relevant native bootstrap/state/lifecycle and package lanes |
| Desktop playback extraction | Relevant native lifecycle, cancellation, cleanup, retirement and safety evidence; required qualification unchanged |
| Release/updater tooling | Existing contract, negative and hosted proofs as applicable; exact artifact/authority and fixture isolation preserved |
| Frontend file grouping | Typecheck, lint, format, unit/browser/bundle tests as applicable and canonical lanes |
| Final baseline | Full canonical validation and exact Windows package/updater qualification |

Record base SHA, tested HEAD/tree, clean/dirty status, OS/toolchain, command/CWD, exit status, selected/skipped lanes and reasons, relevant logs, actual test counts and artifact hashes when applicable. Do not label incomplete, timed-out, skipped or zero-match checks as pass. Check the required aggregate status against the exact source identity expected by the owning CI contract; a green unrelated run is insufficient.

Illustrative local commands after verified cutover (not substitutes for canonical packaging or CI):

```powershell
cargo metadata --locked --format-version 1
cargo tree --locked -p sky_desktop_shell --target x86_64-pc-windows-msvc -e normal,build
cargo tree --locked -p sky_desktop_shell --target x86_64-pc-windows-msvc -e features
cargo fmt --all --check
cargo xtask check all
cargo xtask bindings check
```

Use equivalent commands before migration through the existing approved resolver with `--manifest-path rust/Cargo.toml` where needed. Compare production and qualification with matching package selection, target and feature flags, not merely these illustrative default commands. Follow active packaging/CI/release documents for Windows and fixture commands. Do not use production keys, publish artifacts, promote metadata or execute physical-input acceptance without the applicable explicit authorization. If required evidence cannot be collected, record a blocker; do not substitute mock evidence or call the baseline accepted.

### 10.1 Final acceptance gates

- [ ] One root virtual workspace, one live lockfile, eight preserved members/package identities and no active stale topology paths.
- [ ] Compiler discovery and trusted verification pass, including override-negative tests; isolated output/fixture paths remain correct.
- [ ] Locked dependency resolution, relevant effective features and actual distribution build invocation preserved; `dist` unchanged; no test-feature leakage.
- [ ] Existing architecture, unsafe, security and hard safety rules remain effective; inherited/aliased/target-specific checks covered.
- [ ] Accepted extractions have clear ownership and no unauthorized public API, state, lock, lifetime or wire changes; justified residual large modules allowed.
- [ ] Relevant tests and selectors reconciled, actual intended execution verified, bindings and generated contracts correct.
- [ ] Canonical full validation and exact-source required hosted status complete; classifier remains conservative.
- [ ] Exact Windows NSIS candidate and applicable install/launch/uninstall, resource inventory, app-data preservation and synthetic updater qualification complete; actual historical-release qualification only where explicitly required.
- [ ] Existing release authority, trust, permissions, exact-byte handoff, signing/malware/SBOM/attestation and recovery contracts remain intact with applicable nonproduction proof; no unauthorized publication.
- [ ] Live documents reflect accepted structure; deferred findings and exceptions recorded; owner accepts baseline. Separate performance engineering may then begin.

## 11. Governance, documentation and rollback

Machine-enforce objective dependency, unsafe, feature isolation, generated-contract, safety and build/release requirements using current xtask/CI mechanisms. Fix checks affected by relocation or inheritance and test their failure modes. Review-enforce cohesion, naming, size guidance, test placement and abstraction quality. No new generalized analyzer, path provider, metrics database, universal lint or mandatory API counter by default. A new automated check requires a concrete failure mode and maintenance justification.

Keep `AGENTS.md` a concise router, `CONTRIBUTING.md` the developer workflow, `docs/architecture.md` the architecture owner, and `docs/INDEX.md` the documentation map. Add a compact source-organization policy to an existing owning document, or use `docs/source-organization.md` only if a distinct owner improves navigation. Do not duplicate authority. This SPEC, baseline, implementation notes, review and acceptance evidence are planning/historical artifacts, not four new permanent contracts. Consolidate approved lasting rules into owning documents after acceptance and archive superseded plans per existing convention.

| Risk | Response |
|---|---|
| Missed current path or classifier lane | Stop, fix coherent topology slice, verify required lanes and exact package; revert the whole slice if necessary |
| Fixture path or version/source mismatch | Preserve same-source binding and isolated cleanup; correct fixture, do not add fallback history framework |
| Feature or dependency drift | Revert offending inheritance change; separate real dependency correction |
| Lost test coverage due to selectors | Reconcile discovery and exact counts before accepting extraction |
| State/safety/contract or release authority changes | Stop and revert affected slice; require separate design review for intentional change |
| Toolchain or output overrides | Fail closed in trusted contexts; retain verification and containment |
| Excessive governance or scope | Defer optional slices; simplify conventions; require specific justification |
| Concurrent release or overlapping changes | Coordinate with owner; pin/rebase and revalidate actual source; do not interfere with active release transaction |

Rollback to the last accepted checkpoint through normal Git review; retain evidence. No force-push, deletion of historical assets, weakened tests, broadened exceptions or bypassed required checks to manufacture success.

## 12. Review resolution matrix

All 15 findings from Independent SPEC Review are addressed in this revision at the design level. Implementation proof and owner approval are still pending. Prior classification: 9 major (P1), 5 moderate (P2), 1 minor (P3).

| Finding | Severity | Revision 0.2 resolution | Section |
|---|---|---|---|
| R01 | P1 | Minimal Phase 1 baseline; just-in-time subsystem analysis | 9.1 |
| R02 | P1 | Compare pin-only alternative; root workspace selected with feasibility checkpoint | 3.1 |
| R03 | P1 | Correct same-source synthetic bridge; no historical framework by default | 8.1 |
| R04 | P1 | Consumer-local feature requirements; equivalent declared/resolved/build evidence | 5.2 |
| R05 | P2 | One normal-module 500/800 guidance; preserve scoped existing limits | 6.1 |
| R06 | P2 | Simple physical LOC triage; no mandatory AST parser | 6.1 |
| R07 | P3 | Flexible inline/sibling tests; verify selectors and actual execution | 6.2 |
| R08 | P1 | Physical extraction first; ownership redesign outside default scope | 2.1, 7 |
| R09 | P2 | Visibility by actual callers and invariants, DTO exception | 5.3 |
| R10 | P1 | Priority A, conditional B, deferred C; no mandatory exhaustive split | 7.1, 9 |
| R11 | P1 | Direct local Cargo; trusted pre-build pin verification | 4.2 |
| R12 | P2 | Simple validated paths; metadata only where useful | 4.3 |
| R13 | P1 | Objective checks only, reuse xtask; qualitative review | 5.2, 11 |
| R14 | P2 | Integrate lasting rules into owning docs; archive plans | 11 |
| R15 | P1 | Applicable canonical lanes per slice; full topology/final checks | 10 |

## 13. References and remaining unknowns

Repository evidence at the pinned research baseline:

- [Repository snapshot](https://github.com/pumni/Sky-Auto-Player/tree/5c79c8cf604072f331820126223188161d17d7b1), [workspace manifest](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/rust/Cargo.toml), [architecture](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/docs/architecture.md).
- [Updater fixture source identity](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/rust/xtask/src/updater_fixture.rs#L2028-L2034), [worktree and lockfile mutation](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/rust/xtask/src/updater_fixture.rs#L2142-L2218), [architecture audit](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/rust/xtask/src/audits/architecture.rs).
- [CI topology](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/docs/ci-validation-topology.md), [toolchain policy](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/docs/rust-toolchain-policy.md), [Tauri packaging](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/docs/v4-tauri-packaging.md), [release authority](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/docs/v4-release-authority.md), [playback verification](https://github.com/pumni/Sky-Auto-Player/blob/5c79c8cf604072f331820126223188161d17d7b1/docs/playback-verification.md).

Official references (tool semantics; local conventions remain project decisions):

- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html), [specifying dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html), [resolver](https://doc.rust-lang.org/cargo/reference/resolver.html), [features](https://doc.rust-lang.org/cargo/reference/features.html), [profiles](https://doc.rust-lang.org/cargo/reference/profiles.html).
- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html), [Cargo tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html), [compiler artifact messages](https://doc.rust-lang.org/cargo/reference/external-tools.html), [Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html), [Rustup overrides](https://rust-lang.github.io/rustup/overrides.html).
- [Rust package layout](https://doc.rust-lang.org/cargo/guide/project-layout.html), [module files](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html), [visibility](https://doc.rust-lang.org/reference/visibility-and-privacy.html), [test organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html), [Tauri project structure](https://v2.tauri.app/start/project-structure/).
- Reference designs: [Codex workspace](https://github.com/openai/codex/blob/main/codex-rs/Cargo.toml), [Rust-analyzer architecture](https://rust-analyzer.github.io/book/contributing/architecture.html), [Zed workspace](https://github.com/zed-industries/zed/blob/main/Cargo.toml). The 500/800 guidance is a local proposal inspired by the historical Codex convention discussed in prior research, not a claim about the current Codex AGENTS.md or a Rust language requirement.

**Outstanding evidence:** full current-source path inventory, resolved/compiled production feature comparison, actual locked Tauri Cargo invocation, current canonical validation status and Windows package qualification; detailed API/lock maps only before relevant extraction. None were executed as part of this design revision. The independent review was a critical second pass by the same assistant, not external sign-off.

**Next decision:** owner reviews and approves revision 0.2 (or requests corrections), then authorizes Phase 1 Foundation & Baseline. Do not begin source migration or claim acceptance from this document alone.

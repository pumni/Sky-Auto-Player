# PR3 Work Order — shorten the ordinary-CI critical path

Status: authoritative implementation specification for PR3.

Base commit: `33e0638941ba7a06c58f6ae361c6929b1e8eecf9` (merged PR #191).

## Goal

Reduce ordinary CI wall-clock latency without changing the release trust model, current-candidate byte identity, classifier taxonomy, or workflow count.

PR1 simplified the control plane. PR2 established:

`ONE SOURCE SHA -> ONE CURRENT CANDIDATE BUILD -> MANY QUALIFIERS CONSUME THE SAME BYTES`.

PR3 must preserve that invariant and shorten the critical path by running independent expensive work in parallel.

## Baseline from main CI run #825

Run: `34502752316`
Source SHA: `33e0638941ba7a06c58f6ae361c6929b1e8eecf9`
Result: success.

Observed wall-clock structure:

- workflow start: about `16:32:42Z`
- current candidate job: `16:33:00Z` -> `16:40:43Z` (~7m43s)
- current `tauri build` step: `16:34:40Z` -> `16:39:57Z` (~5m17s)
- package qualifier: `16:40:47Z` -> `16:43:01Z` (~2m14s)
- Windows compatibility/unit job: `16:32:59Z` -> `16:44:59Z` (~12m)
- updater qualifier: `16:40:47Z` -> `16:49:53Z` (~9m06s)
- updater step `Build throwaway signed fixture and qualify packaged Tauri updater rotation`: `16:43:22Z` -> `16:49:22Z` (~6m)
- required gate completed about `16:49:59Z`
- total full-matrix elapsed: about 17m17s.

The updater fixture build is therefore measured critical-path work. If it is moved in parallel, the existing combined Windows validation job can become the next critical path. PR3 addresses both facts and nothing broader.

## Scope

PR3 has exactly two latency changes:

1. prebuild one updater bridge fixture in parallel with the current candidate;
2. move frontend/browser validation off the combined Windows native-validation job.

No general CI rewrite, no release topology rewrite, and no broad cache redesign.

## Hard boundaries

Do not:

- change classifier public outputs or path taxonomy;
- add another workflow file;
- change the exact required status name `Sky Auto Player — required CI gate`;
- build a second current candidate;
- weaken `profile.dist` (`opt-level=3`, thin LTO, `codegen-units=1` remain unchanged);
- change production/rehearsal release authority, signing, provenance, attestations, immutable-release rules, source-SHA binding, metadata promotion, or runner/environment isolation;
- move production updater private keys into CI artifacts, outputs, logs, repository files, or workspace-persistent state;
- put candidate test private keys into artifacts;
- put old-root negative-test private keys into artifacts;
- introduce merge queue, reusable release workflow abstraction, or release state-machine cleanup;
- redesign caches beyond small changes strictly required by the runner split;
- delete qualification coverage to obtain a faster run.

The active workflow set must remain exactly:

- `.github/workflows/ci.yml`
- `.github/workflows/pages.yml`
- `.github/workflows/release-v4.yml`
- `.github/workflows/rehearse-v4.yml`

## Part A — prebuilt updater bridge

### Topology

Add one ordinary-CI producer job, conceptually `updater_bridge`.

It runs when `updater_required == true` and should depend only on path classification (`changes`) so that it can begin in parallel with the current-candidate producer and static/release-contract lanes.

The ordinary updater consumer must depend on both:

- the exact current-candidate producer;
- the exact updater-bridge producer;

plus the existing prerequisite gates needed for safe qualification.

The package consumer continues to depend only on the current candidate, not on the bridge.

### Bridge semantics

The bridge is a throwaway **test fixture**, not a release candidate and not release authority.

Build exactly one bridge NSIS installer per updater-sensitive CI run. It must be built from the same exact source SHA as the workflow run and with `tauri-update-fixture` enabled.

The bridge build must be independent of:

- the current candidate updater public key;
- the old-root negative-test key;
- the loopback port selected later by the updater consumer.

This independence is what allows the bridge producer to run in parallel with the candidate producer.

### Fixture-only runtime configuration seam

Today the fixture port and public roots are compile-time `option_env!` inputs. PR3 must replace that requirement for the updater fixture with a bounded runtime seam compiled **only** under `cfg(feature = "tauri-update-fixture")`.

The runtime seam must accept only the information necessary for the fixture:

- a loopback TCP port;
- one or more updater public-key file paths, in trust-order;
- the existing fixture-only `new-only` selection for the old-root rejection phase.

Exact flag names are implementation details, but they must be self-test/fixture scoped and explicit.

Security constraints for the fixture seam:

- production builds (`not(feature = "tauri-update-fixture")`) continue to use the existing fixed production metadata endpoints and fixed production updater roots;
- runtime endpoint injection must not accept an arbitrary URL or host;
- fixture host remains hard-fixed to `127.0.0.1`;
- channel path remains hard-fixed to the existing `/stable` or `/beta` fixture path;
- only a validated numeric TCP port may vary;
- public-key files must be regular bounded files, non-empty, and must reject obvious private-key material;
- missing/invalid fixture runtime arguments fail closed; do not silently fall back to the build-time dummy fixture configuration;
- `new-only` may only select from the already supplied fixture public keys;
- no equivalent runtime override may exist in production builds.

The bridge build may use a disposable generated updater key only to satisfy Tauri bundle generation. That private key must live under `RUNNER_TEMP`, be deleted/cleaned, and must never be uploaded. Its public key is not the runtime trust root used by qualification.

### Bridge artifact contract

Upload a clean artifact containing exactly:

- one bridge NSIS installer;
- one `bridge.json` contract.

Do not upload the bridge build signing private key, public key, `.sig`, target tree, dependency caches, source copies, or logs.

`bridge.json` must contain at minimum:

- schema version;
- exact source SHA;
- bridge version;
- installer filename;
- installer SHA-256;
- selected built-in catalog sentinel stable ID;
- sentinel content SHA-256.

Use relative filenames only. No runner absolute paths.

The bridge producer must stage the exact two-file contract into a clean artifact directory and validate multiplicity before upload.

If the bridge producer mutates the built-in catalog source to create the N sentinel, restore tracked source bytes before job completion and fail if the source tree cannot be restored exactly.

### Updater consumer

Extend the existing updater harness with an optional provided-bridge interface, analogous to the existing provided-candidate interface.

All provided-bridge fields must be supplied together. A partial provided-bridge request fails closed.

In ordinary CI, the updater consumer must:

1. download the current-candidate artifact from the same workflow run;
2. independently validate/re-hash candidate metadata as PR2 requires;
3. download the updater-bridge artifact from the same workflow run;
4. validate exact bridge multiplicity, source SHA, version, filename, installer SHA-256, and sentinel contract;
5. generate the old-root negative-test keypair locally in this consumer only;
6. supply the old public key plus the candidate public key to the bridge at runtime;
7. choose a disposable loopback port at runtime;
8. install the exact prebuilt bridge fixture;
9. run the existing N -> N+1 update using the exact downloaded current-candidate installer/signature;
10. preserve all existing shutdown-safety, user-data, built-in-catalog, loopback HTTP, same-byte, and old-root rejection checks;
11. run the negative phase with the same preserved bridge binary in fixture `new-only` mode;
12. require the real Tauri `Update::download` path to fetch the candidate and reject the old-root signature.

In **provided-bridge mode**, the updater consumer must execute zero `tauri build` commands. It may compile small Rust/xtask helpers if existing checks require them, but it must not rebuild the bridge or current candidate.

The existing non-provided-bridge fallback may remain for release/rehearsal qualification if those pipelines currently rely on the harness building its own throwaway bridge. That fallback must preserve current behavior and security semantics.

### Private-key locality

PR3 must preserve three distinct key localities:

- current-candidate test signing private key: candidate producer only;
- bridge-build disposable private key: bridge producer only, if needed only to satisfy Tauri bundle generation;
- old-root negative-test private key: updater consumer only.

No one of these private keys may cross a job boundary.

## Part B — split web/browser validation from Windows native validation

### Motivation

Run #825's combined Windows validation lasted about 12 minutes. It serialized Rust verification, Bun/Chromium setup, desktop web checks, desktop native checks, and cache finalization.

Once updater fixture compilation is parallelized, this combined job is likely to become a larger share of the required-gate path.

### Ubuntu desktop-web job

Create a separate job, conceptually `desktop_web`, on `ubuntu-24.04`.

Run it when `desktop_required == true`.

It owns the platform-neutral desktop web work:

- exact source checkout;
- Bun 1.4.0 setup;
- `bun install --frozen-lockfile` in `desktop`;
- `bun run check`;
- when `desktop_e2e_required == true`, verify the local Playwright CLI version against `package.json`, install Chromium using the pinned local Playwright CLI, and run `bun run test:e2e`.

When `desktop_e2e_required == false`, do not install Chromium and do not run browser E2E.

Do not run Windows-native Tauri/Rust checks in this Ubuntu job.

### Windows native-validation job

Keep Windows validation for work that is genuinely Windows/native:

- `cargo xtask check rust` when `rust_required == true`;
- the native/Tauri portion of desktop validation when `desktop_required == true`.

The Windows job must no longer:

- set up Bun for desktop verification;
- run `bun install` for desktop verification;
- install Chromium;
- run Playwright/browser E2E;
- run frontend type/build checks already owned by `desktop_web`.

Do not remove native/Tauri coverage.

### Xtask contract

Keep `cargo xtask check desktop` as the existing full local developer contract: it must still run both web/browser (unless its existing explicit browser-skip behavior is requested) and native desktop checks.

Add one explicit CI-oriented native-only check mode, preferably `cargo xtask check desktop-native`, which executes the native/Tauri subset without running Bun/frontend/browser work.

Do not replace the canonical local `check desktop` command with a CI-only mode.

Static/self-tests must prove that:

- `check desktop` still contains the full expected validation;
- `check desktop-native` does not invoke Bun or browser E2E;
- CI uses `desktop-native` on Windows;
- CI runs web/browser validation on Ubuntu;
- desktop E2E is still conditional on `desktop_e2e_required`.

## Aggregate required gate

Preserve the exact check display name:

`Sky Auto Player — required CI gate`

Update aggregate dependencies and expected-skip logic so that:

- `updater_bridge` must succeed when `updater_required == true`, otherwise it must be skipped;
- `desktop_web` must succeed when `desktop_required == true`, otherwise it must be skipped;
- candidate/package/updater/release/static/supply-chain/site semantics remain unchanged;
- a failed new producer/consumer lane cannot be hidden by skip propagation;
- aggregate continues to use fail-closed result inspection.

Do not create additional required branch-status names.

## Actions and permissions

Any newly introduced GitHub Action reference must be pinned to a full commit SHA.

Keep job permissions least-privilege. The new bridge and desktop-web jobs should require no write permissions beyond artifact upload metadata automatically needed by the action/runtime.

Do not add OIDC or attestation permissions to ordinary CI.

## Contract/self-test updates

Update `rust/xtask/src/checks.rs` and/or focused PowerShell self-tests so architecture is enforced semantically, not merely by job names.

At minimum guard:

- one current-candidate producer;
- one bridge producer only when updater is required;
- bridge producer and candidate producer both depend directly on `changes` rather than serializing behind each other;
- updater consumer depends on both producer artifacts;
- updater consumer's provided-bridge path contains no `tauri build`;
- bridge artifact has exact bounded contents and source-SHA/hash binding;
- no private key file is uploaded in candidate or bridge artifacts;
- fixture runtime port is loopback-only and fixture-feature-only;
- fixture runtime public-root injection is fixture-feature-only and fails closed;
- production metadata endpoints and production public roots remain fixed;
- desktop web/browser CI runs on Ubuntu;
- Windows native validation has no Bun/Chromium/browser setup;
- `cargo xtask check desktop` remains the full local contract;
- exact aggregate gate name remains unchanged;
- active workflow count remains four.

Avoid brittle tests that depend on harmless YAML ordering beyond the dependency relationships that matter.

## Acceptance criteria

PR3 is acceptable only when all of the following are demonstrated on the exact final PR head:

1. Full CI is green, including the exact required aggregate gate.
2. Classifier self-change causes expected full validation.
3. There is exactly one current-candidate `tauri build`.
4. Candidate package/updater consumers still report the same current installer SHA-256.
5. One updater bridge fixture is built in its producer; ordinary updater consumer performs zero `tauri build` operations.
6. Candidate producer and bridge producer overlap in wall-clock execution in the same full CI run.
7. Bridge artifact contains only the bridge installer and `bridge.json`.
8. Bridge artifact contains no private key, public key, detached signature, target directory, or cache material.
9. Bridge `source_sha` equals exact workflow source SHA and its installer hash is independently revalidated by updater consumer.
10. Runtime fixture configuration accepts only loopback port + bounded public-key files and cannot affect production builds.
11. Old-root rejection still downloads the exact current candidate bytes through real Tauri updater verification and rejects the old-root signature.
12. User-data and built-in-catalog N -> N+1 preservation/replacement evidence remains intact.
13. Desktop web checks execute on Ubuntu; Chromium/browser E2E only runs when `desktop_e2e_required` is true.
14. Windows native validation runs no Bun/Chromium/browser work and retains Rust/native/Tauri coverage.
15. `cargo xtask check desktop` remains valid as the full local developer check.
16. Production/rehearsal release workflows and security invariants are not weakened.
17. Active workflow count remains exactly four.
18. Required gate display name is byte-for-byte unchanged.
19. A full PR run records before/after timing against run #825. The implementation should materially improve the ~17m17s baseline; if the representative full run is not below 15 minutes excluding unusual runner queue delay, stop and analyze the remaining critical path before declaring PR3 accepted.

The under-15-minute point is a performance acceptance target, not permission to delete or weaken tests.

## Validation expected from Codex

Before handing the PR back for review, run focused local checks practical on the development machine, including at least:

- candidate/classifier self-tests;
- new bridge contract/runtime-fixture self-tests;
- updater harness parsing/tests;
- `cargo fmt --check`;
- `cargo test --manifest-path rust/xtask/Cargo.toml --locked`;
- relevant Tauri/native updater tests;
- `cargo xtask check static`;
- `git diff --check`.

Do not claim hosted Windows/Ubuntu topology acceptance from local tests. Push the branch and let GitHub provide final runtime evidence.

## Review evidence to report

On completion report:

- final commit SHA;
- full CI run ID;
- candidate installer SHA-256 seen by package and updater;
- bridge installer SHA-256 seen by producer and updater consumer;
- count/location of current-candidate `tauri build` commands;
- count/location of bridge `tauri build` commands;
- updater consumer confirmation that it performs zero builds;
- candidate producer start/end;
- bridge producer start/end;
- updater consumer start/end;
- Windows native job start/end;
- Ubuntu desktop-web job start/end;
- exact required-gate completion time;
- measured full-run elapsed time versus the run #825 ~17m17s baseline.

## Out of scope follow-up

After PR3 is merged, use measured post-PR3 timings before considering any PR4. Possible later work such as cache-key redesign, release-contract test decomposition, or candidate build optimization is explicitly not part of this PR.

## Round 2 measured note

Run `34511947829` showed that the independent non-production updater key-rotation contract consumed about 137 seconds inside `updater_e2e`, while the provided-bridge runtime qualification itself took about 43 seconds. This round extracts that contract into the parallel `updater_contract` lane keyed only by `updater_required`; updater runtime qualification and its coverage remain unchanged.

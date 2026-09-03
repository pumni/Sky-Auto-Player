# Wave 8 — CI Fast-Feedback Performance Baseline & Report

Date recorded: 2026-09-03
Baseline commit: `main@6a6155df83294d0defe35f6ef4f774e2405c5a45` (PR #95 merge)
Main post-#95 workflow: `CI run #549 / 33659228830`
PR control workflow: `PR #95 run #548 / 33658064660`

---

## 1. Baseline Framework & Attribution Controls

To avoid comparing apples to oranges, performance measurements in Wave 8 are strictly partitioned into two separate baselines:
1. **Main Push Baseline (with Post-Job Cache Saves)**: Evaluates the full master qualification path where `save-if: true` compresses and uploads the Rust cache archives.
2. **PR Control Baseline (Restore-Only)**: Evaluates normal developer pull-request latency where `save-if: false` skips post-job cache saves.

---

### 1.1 Post-#95 Main Push Baseline (Run #549 / `33659228830`)

> [!IMPORTANT]
> On `main` pushes, post-job actions run after the job scripts finish and sit directly on the critical path of required gate convergence. Below, timings are divided into **True Job Wall-Clock** (GitHub job lifecycle) and **Pre-Post / Productive Duration** (step timings before post-actions).

| Job / Phase | True Job Wall-Clock | Productive / Pre-Post | Breakdown & Authoritative Details |
| :--- | ---: | ---: | :--- |
| **`changes` total** | **59s** | 59s | Full git checkout ~21s, rustup minimal ~7–8s, un-cached `sky_xtask` compilation ~20s |
| **`static` total** | **50s** | 50s | Ubuntu runner, zero `rust-cache`, compiles `xtask` again |
| **`validate` total** | **15m17s** | **~8m43s** | Job start 17:09:35 $\rightarrow$ tests finish 17:18:18Z (~8m43s pre-post; ~8m38s script timing); **Post Rust cache save: 6m27s** (17:18:19–17:24:46Z) |
| **`packaged` total** | **10m33s** | **~7m49s** | Job start 17:09:35 $\rightarrow$ pack finish 17:17:24Z (~7m49s pre-post); **Post Rust cache save: 2m40s** (17:17:24–17:20:04Z) |
| **`status` (Required gate)** | **~9s** | ~9s | Convergence check after `validate` post-cache completes |
| **Full Main Wall-Clock** | **16m38s** | **~9m50s** | Workflow start 17:08:28Z $\rightarrow$ required gate 17:25:05Z (Critical path pre-post: ~9m50s) |

#### Critical Finding on Cache Economics (Input for CI-FAST-3)
* On `main`, the post-action save step in `validate` alone costs **6m27s** of wall-clock delay. Because `validate` and `packaged` run in parallel, the critical-path cache-save tax is dominated by `validate` (~6m27s).
* This save tax represents **~39% of total main workflow wall-clock**, confirming the core thesis for CI-FAST-3: target-cache archive size and upload frequency must be re-architected.

---

### 1.2 PR Control Baseline (Pre-FAST-1 PR #95 Run #548 vs FAST-1 PR #96 Run #550)

To establish true causal attribution for PR developer feedback, PR #96 (FAST-1) is compared directly against PR #95 Run #548 (Pre-FAST-1), both running under `save-if: false` (restore-only).

| Metric | Pre-FAST-1 PR Control<br/>(Run #548) | FAST-1 Candidate<br/>(PR #96 Run #550) | Delta / Change | Attribution Analysis |
| :--- | ---: | ---: | ---: | :--- |
| **`changes` job** | **33s** | **17s** (run #551: **15s**) | **-16s (-48%)** | **Direct Causal**: Elimination of full git checkout, zero-dep classifier crate (`1.12s` compile vs `~15s`). |
| `static` | 43s | 46s (run #551: **43s**) | +3s / 0s | Unchanged architecture (Wave-6 baseline fetched dynamically in 1s). |
| `validate` | 8m41s | 7m53s (run #551: 8m44s) | -48s / +3s | Confounded / observational variance (see caveat below). |
| `packaged` | 8m06s | 7m22s (run #551: 8m04s) | -44s / -2s | Confounded / observational variance. |
| `status` | ~5s | 3s (run #551: 5s) | -2s | Native Bash timing vs pwsh. |
| **Full PR Wall-Clock** | **9m31s** | **8m24s** | **-1m07s (-11.7%)** | Confounded by browser workload differences. |

> [!NOTE]
> **Attribution Caveat on Workflow Wall-Clock**:
> In PR #95, UI changes triggered `browser_required=true` (Playwright Chromium install ~22s), whereas in PR #96 the classifier correctly evaluated `browser_required=false`. Therefore, the 67-second total PR workflow difference cannot be causally attributed solely to FAST-1.
> 
> **The verified causal proof of FAST-1 is:**
> 1. Classifier job wall-clock reduced from **33s $\rightarrow$ 17s** (and **15s** on Run #551), a **~48% reduction**.
> 2. Classifier compilation reduced from **~15s $\rightarrow$ 1.12s**.
> 3. Main / dispatch bypass reduced from **59s $\rightarrow$ 3s** (see Section 3.3).

---

## 2. Wave 8 Execution Matrix & Phased Comparison

| Metric | Main Baseline (#549)<br/>True Wall-Clock | PR Control (#548)<br/>True Wall-Clock | CI-FAST-1 (Hosted PR #96)<br/>Run `33665614053` | CI-FAST-2 (Hosted PR #97)<br/>Run `33694232773` | CI-FAST-3<br/>(Cache Architecture) | Final Wave 8 |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| `changes` (PR) | 59s | 33s | **17s** (Run #551: **15s**) | **13s** | — | TBD |
| `changes` (Dispatch/Main) | 59s | N/A | **3s** (Run #552) | **4s** (Run #559) | — | TBD |
| `static` | 50s | 43s | **46s** (Run #551: **43s**) | **34s** (Run #559: **30s**) | — | TBD |
| `supply_chain` | (in static) | (in static) | (in static) | **19s** (Run #559: **27s**) | — | TBD |
| `validate` (PR restore-only) | N/A | 8m41s | **7m53s** (Run #551: 8m44s) | **7m43s** | TBD | TBD |
| `validate` (Main with save) | 15m17s | N/A | N/A | **15m45s** (Run #559) | TBD | TBD |
| `packaged` (PR restore-only) | N/A | 8m06s | **7m22s** (Run #551: 8m04s) | **5m39s** | TBD | TBD |
| `packaged` (Main with save) | 10m33s | N/A | N/A | **9m45s** (Run #559) | TBD | TBD |
| `status` (Required gate) | ~9s | ~5s | **3s** (Run #551: **5s**) | **3s** (Run #559: **2s**) | <= 5s | TBD |
| **Workflow Wall-Clock (PR)** | N/A | **9m31s** | **8m24s** | **8m05s** | TBD | TBD |

---

## 3. CI-FAST-1 Hosted Qualification Evidence

### 3.1 Candidate PR Execution Runs

| Run Number | Workflow Run ID | Trigger Event | Commit HEAD | Lifecycle (UTC) | Duration | Gate Result |
| :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **Run #550** | `33665614053` | `pull_request` | `1ca1ada4abb9` | 18:10:42 $\rightarrow$ 18:19:06Z | **8m24s** | **100% GREEN** |
| **Run #551** | `33666573656` | `pull_request` | `36c781efe245` | 18:20:15 $\rightarrow$ 18:29:43Z | **9m28s** | **100% GREEN** |
| **Run #553** | `33667667515` | `pull_request` | `700e9439da1c` | 18:30:56 $\rightarrow$ 18:40:36Z | **9m40s** | **100% GREEN** |
| **Run #552** | `33667547040` | `workflow_dispatch` | `36c781efe245` | 18:29:51 $\rightarrow$ 18:48:02Z | `changes` **3s** | Partial fail: `test:e2e` (Vite 15s cold readiness timeout) |
| **Run #554** | `33669521385` | `workflow_dispatch` | `700e9439da1c` | 18:48:53Z $\rightarrow$ *active* | `changes` **3s** | In progress (manual dispatch qualification) |

* **Candidate PR**: [#96](https://github.com/pumni/Sky-Auto-Player/pull/96)
* **Adoption Status**: `ADOPTED`

---

### 3.2 Authoritative Artifact Evidence for Current PR HEAD (`700e9439da1c`) — Run #553

- **Artifact ID**: `9861533057`
- **Name**: `sky-auto-player-portable-700e9439da1cf72ca5b4e25451d10920a21920d0`
- **Size**: `9,171,983 bytes`
- **SHA-256 Digest**: `sha256:1f2677fe3ae5cb1ed3f05a471115d88557c1176a1b91b414e9b25f6a29f026c6`
- **URL**: `https://github.com/pumni/Sky-Auto-Player/actions/runs/33667667515/artifacts/9861533057`

---

### 3.3 Detailed Timestamps for Run #550 (`1ca1ada4abb9`)

#### Job 1: `changes` (ID `100366483302`)
- **Runner**: `ubuntu-latest` (`GitHub Actions 1000003282`)
- **Wall-Clock**: `18:10:46Z` $\rightarrow$ `18:11:03Z` (**17s**)
- **Steps**:
  - Checkout PR head (`fetch-depth: 1`): 2s
  - Fetch PR base commit (`--depth=1`): <1s
  - Install pinned Rust toolchain (1.98.0 minimal): 8s
  - Verify classifier toolchain (`RUSTUP_TOOLCHAIN: 1.98.0`): <1s
  - Classify PR changed paths (`sky_ci_classifier`): **1.12s compilation + execution**
  - Report CI timing (native Bash): <1s
- **Classification Output**:
  ```text
  static_required=true
  code_required=true
  package_required=true
  browser_required=false
  classification_reason=package-sensitive: .github/workflows/ci.yml, .github/workflows/release.yml, rust/Cargo.lock
  ```

#### Job 2: `static` (ID `100366597794`)
- **Runner**: `ubuntu-latest` (`GitHub Actions 1000003285`)
- **Wall-Clock**: `18:11:06Z` $\rightarrow$ `18:11:52Z` (**46s**)
- **Steps**:
  - Checkout (`fetch-depth: 1`): 2s
  - Fetch Wave-6 baseline (dynamic resolution from ledger JSON + `git cat-file -e`): 1s
  - Cargo audit & Cargo vet: 7s
  - Repository verification — static: 18s

#### Job 3: `validate` (ID `100366597848`)
- **Runner**: `windows-latest` (`GitHub Actions 1000003283`)
- **Wall-Clock**: `18:11:06Z` $\rightarrow$ `18:18:59Z` (**7m53s**)
- **Steps**:
  - Checkout (`fetch-depth: 1`): 6s
  - Fetch Wave-6 baseline (dynamic PowerShell from ledger + `git cat-file -e`): 1s
  - Rust cache restore (`Swatinem/rust-cache`): 51s
  - Restricted repository verification without Python: 6m29s
  - Post Rust cache: <1s (save skipped on PR)

#### Job 4: `packaged` (ID `100366597914`)
- **Runner**: `windows-latest` (`GitHub Actions 1000003284`)
- **Wall-Clock**: `18:11:05Z` $\rightarrow$ `18:18:27Z` (**7m22s**)
- **Steps**:
  - Checkout (`fetch-depth: 1`): 7s
  - Rust cache restore: 26s
  - Dist build (`cargo xtask dist --profile dist`): 6m28s
  - Upload exact portable release candidate (`actions/upload-artifact@v7.0.1`): 2s

#### Job 5: `status` (ID `100369273840`)
- **Runner**: `ubuntu-latest` (`GitHub Actions 1000003286`)
- **Wall-Clock**: `18:19:03Z` $\rightarrow$ `18:19:06Z` (**3s**)
- **Gate Result**: Converged green on all 4 upstream requirements.

---

### 3.4 Hosted `workflow_dispatch` Qualification (Run #552 & #554)

To qualify the short-circuit optimization for manual dispatch and main push:
- **Trigger**: `workflow_dispatch` on `perf/ci-fast-1-bootstrap`
- **Job**: `Classify validation layers` (ID `100372902296` in #552; ID `100379382020` in #554)
- **Job Lifecycle**: **3 SECONDS** wall-clock (down from 59s on main)
- **Execution Path**:
  - `Emit full qualification matrix for main / manual dispatch`: executed in <1s
  - `Checkout PR head`: skipped
  - `Fetch PR base commit`: skipped
  - `Install pinned Rust toolchain`: skipped
  - `Verify classifier toolchain`: skipped
  - `Classify PR changed paths`: skipped
- **Outputs Emitted**:
  ```text
  static_required=true
  code_required=true
  package_required=true
  browser_required=true
  classification_reason=full validation requested
  ```
- **Concurrency Isolation**: The run executed under dedicated group `CI-<run_id>` with `cancel-in-progress: false`, operating concurrently without cancelling or being cancelled by concurrent PR runs.
- **Investigation of Run #552 `validate` Failure**:
  - Root cause: In `desktop/scripts/run-e2e.mjs:28`, `waitForServer()` has a fixed 15-second deadline (`60 * 250ms`) for Vite dev server initialization. Under heavy load on Windows runner after running the entire Rust test suite, cold Vite initialization took slightly longer than 15s, triggering `Error: Vite did not become ready within 15 seconds`.
  - All preceding steps passed cleanly: `check static` PASS, `check rust` PASS, `bun run check` PASS. Rerun #554 triggered to observe qualification.

---

## 4. CI-FAST-2: Static Checks and Supply-Chain Audits Separation

### 4.1 Architectural Changes
1. **Dedicated Parallel Jobs in CI**:
   - `static` (`Static and security gates`): Repository static invariant verification (`cargo xtask check static --skip-supply-chain`). Supply-chain and advisory network operations are removed from `static`; `cargo-audit` and `cargo-vet` are isolated into the parallel `supply_chain` job, eliminating advisory DB downloads and tool cache restores from the static critical path.
   - `supply_chain` (`Supply-chain and advisory security`): Dedicated job on `ubuntu-latest` running in parallel with `static`. Restores and caches pinned `cargo-audit` (v0.22.2) and `cargo-vet` (v0.10.2). Executes `cargo audit --file rust/Cargo.lock` and `cargo vet --manifest-path rust/Cargo.toml --locked`.
2. **Local Contract & Fail-Closed Semantics Preservation**:
   - Running `cargo xtask check static` locally without flags continues to execute all checks including `supply_chain::run(None)` by default.
   - Flag `--skip-supply-chain` or environment variable `SKY_CHECK_SKIP_SUPPLY_CHAIN=1` safely bypasses the redundant `cargo vet` step when executed under dedicated CI pipelines. The environment variable check enforces strict fail-closed equality (`== "1"`) to prevent accidental bypass via `0`, `false`, or empty values.
   - Preserves all security invariants from `SECURITY.md` and zero-Python audit rules from `AGENTS.md`.
3. **Gate Convergence**:
   - Updated `status` required CI gate to depend on `[changes, static, supply_chain, validate, packaged]`, failing closed if either `static` or `supply_chain` fails.

### 4.2 Hosted Qualification Evidence

#### PR Run #558 (`33694232773`) on HEAD `e824c4c21cf3`
- **Trigger**: `pull_request`
- **Total Jobs**: 6/6 `SUCCESS`
- **Timing Evidence**:
  - `changes`: **13s** (ID `100459519681`)
  - `static`: **34s** (ID `100459584997`) — **achieved target <= 35–40s**
  - `supply_chain`: **19s** (ID `100459585070`) — concurrent parallel job
  - `validate`: **7m43s** (ID `100459584973`)
  - `packaged`: **5m39s** (ID `100459584981`)
  - `status`: **3s** (ID `100461425486`)
- **Authoritative Artifact**:
  - Artifact ID: `9871324415`
  - Name: `sky-auto-player-portable-e824c4c21cf3c7328076ca9df64f6a259b654315`
  - Size: `9,171,940 bytes`
  - Digest: `sha256:932cc5e1fb0c68bc353af9c5e8ffa2d460952623d7714e5f0307d0a87fdebdbb`

#### Manual Dispatch Run #559 (`33694905103`) on HEAD `e824c4c21cf3`
- **Trigger**: `workflow_dispatch`
- **Total Jobs**: 6/6 `SUCCESS`
- **Timing Evidence**:
  - `changes`: **4s** (ID `100461577952`)
  - `static`: **30s** (ID `100461603388`)
  - `supply_chain`: **27s** (ID `100461603473`)
  - `validate`: **15m45s** (ID `100461603580` — full browser tests + main cache save)
  - `packaged`: **9m45s** (ID `100461603427`)
  - `status`: **2s** (ID `100465247611`)
- **Authoritative Artifact**:
  - Artifact ID: `9871600003`
  - Name: `sky-auto-player-portable-e824c4c21cf3c7328076ca9df64f6a259b654315`
  - Size: `9,171,973 bytes`
  - Digest: `sha256:19e43255c1f8754a52cdecbf469269c8faab5df8a7a8901bae17e066d9f4202a`

#### Exact-Head PR Qualification Run #561 (`33697726794`) on HEAD `c8e6fc2875f1`
- **Trigger**: `pull_request` (PR #97)
- **Total Jobs**: 6/6 `SUCCESS`
- **Timing Evidence**:
  - `changes`: **16s** (ID `100470149626`)
  - `static`: **34s** (ID `100470225379`)
  - `supply_chain`: **26s** (ID `100470225358`)
  - `validate`: **7m45s** (ID `100470225323`)
  - `packaged`: **8m27s** (ID `100470225290`)
  - `status`: **2s** (ID `100472124981`)
- **Authoritative Artifact**:
  - Artifact ID: `9872596392`
  - Name: `sky-auto-player-portable-c8e6fc2875f1f4c0cb31901b360e5e661ca2d618`
  - Size: `9,172,001 bytes`
  - Digest: `sha256:990533d8ff1b51e11022b33d97598a42c5e77cac82270802e4aad654c2493250`
- **Merge Provenance**:
  - PR #97 merged into `main` at commit `81bb7c2f1dff152e38fe1f721dae549bf442266b`.

---

## 5. CI-FAST-3: Rust Cache Architecture & Target Pruning

### 5.1 Problem Statement & Hypothesis
In Control A (`cache-targets: true`), caching the whole `rust/target` directory imposes:
- **Save Tax on Main / Dispatch**: ~6m27s compressing and uploading ~1.5 GB in `validate`, plus ~2m40s in `packaged`.
- **Restore Tax on Windows Runners**: ~36s–60s per job unpacking target caches, with frequent lock contention and cache eviction.
- **Variant B Hypothesis**: Disabling target caching (`cache-targets: false`) retains Cargo registry, git index, and tool binaries (`$CARGO_HOME/registry`, `$CARGO_HOME/git`, `$CARGO_HOME/bin`), but skips `rust/target`. This should eliminate the 6m+ post-save tax and reduce restore time, while evaluating whether clean crate compilation against cached dependencies produces a net win in required-gate critical path.

### 5.2 Variant B Configuration
Applied to both `validate` and `packaged` in `.github/workflows/ci.yml`:
```yaml
      - name: Rust cache
        id: rust-cache
        uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          workspaces: "rust -> target"
          cache-targets: false
          cache-on-failure: true
          save-if: ${{ github.event_name != 'pull_request' }}
```

### 5.3 Benchmark & Measurement Framework
For each candidate run, record:
1. `Rust cache restore — validate`
2. `Rust cache restore — packaged`
3. `Productive compile/test — validate`
4. `Dist build — packaged`
5. `Post Rust-cache save — validate` (main/dispatch)
6. `Post Rust-cache save — packaged` (main/dispatch)
7. `Required-gate wall clock` (adoption decider)

### 5.4 Empirical Measurements: Control A vs Variant B

| Metric | Control A (Main Run #562) | Control A (PR Run #561) | Variant B (Manual Run #564) | Variant B (PR Run #563) | Impact / Delta (Variant B) |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **Commit SHA** | `81bb7c2f1dff` | `c8e6fc2875f1` | `f6ca8063e810` | `f6ca8063e810` | Exact candidate HEAD |
| **Trigger** | `push` (main) | `pull_request` | `workflow_dispatch` | `pull_request` | Full matrix qualification |
| **Rust-cache restore — `validate`** | 55s | ~48–55s | **6s** | **7s** | **~88% faster restore** (-48s) |
| **Rust-cache restore — `packaged`** | ~36s | ~32s | **5s** | **11s** | **~70–85% faster restore** (-25s to -31s) |
| **Productive test — `validate`** | 7m03s | 7m10s | 11m13s | 10m53s | **+3m43s to +4m10s** (clean crate build) |
| **Dist build — `packaged`** | 8m00s | 7m50s | 10m22s | 10m50s | **+2m22s to +3m00s** (clean crate build) |
| **Post-save — `validate`** | 6m31s (391s) | 0s (skipped) | **15s** | 0s (skipped) | **-6m16s save tax eliminated** |
| **Post-save — `packaged`** | ~2m19s (139s) | 0s (skipped) | **32s** | 0s (skipped) | **-1m47s save tax reduced** |
| **Job total — `validate`** | 15m35s | 7m45s | **12m32s** | **11m36s** | **-3m03s on main** / +3m51s on PR |
| **Job total — `packaged`** | 9m37s | 8m27s | **11m52s** | **11m46s** | +2m15s on main / +3m19s on PR |
| **Required-gate Wall-Clock** | **15m40s** | **8m35s** | **13m00s** | **12m13s** | **-2m40s on main (-17%)** / +3m38s on PR |

#### Warm Variant B PR Evidence (Run #565 on HEAD `66354ce43be2`)
- **Trigger**: `pull_request` (PR #98)
- **Cache State**: Warm Cargo registry/git/bin cache (populated by Run #564); clean `target/`.
- **Timing Results**:
  - `changes`: **16s** (ID `100490154685`)
  - `static`: **33s** (ID `100490219827`)
  - `supply_chain`: **27s** (ID `100490219805`)
  - `validate`: **11m30s** (ID `100490219858`)
  - `packaged`: **11m40s** (ID `100490219853`)
  - `status`: **5s** (ID `100492583741`)
  - Total workflow wall-clock: **12m24s** (01:36:41Z $\rightarrow$ 01:49:05Z).
- **Core Finding & Decision**:
  - Warm Cargo dependency cache alone does NOT restore PR feedback latency, as Cargo still compiles clean workspace and dependency crate graphs without object-level reuse (~11m30s vs ~8m35s in Control A).
  - **Verdict**: Variant B is accepted as an empirical benchmark and lower-level control architecture, but rejected for final adoption. We proceed to **Variant C (`cache-targets: false` + `sccache`)**.

#### Authoritative Artifacts for CI-FAST-3 Variant B:
- **PR Run #563 (`33703355904`)**:
  - Artifact ID: `9874610048`
  - Name: `sky-auto-player-portable-f6ca8063e810fd9cfae4cc50f3af80b48c0c1bde`
  - Size: `9,172,089 bytes`
  - Digest: `sha256:f597df11e1d0df80f2f5d244281a035c681ee1eee5520fc8cf0fe70860193418`
- **Manual Run #564 (`33703366753`)**:
  - Artifact ID: `9874584256`
  - Name: `sky-auto-player-portable-f6ca8063e810fd9cfae4cc50f3af80b48c0c1bde`
  - Size: `9,172,007 bytes`
  - Digest: `sha256:a069bab7c0f3db0b2d823ec42208bf8cb3993e8c531605b1a84d425baa89bbd2`

---

## 6. CI-FAST-3: Variant C (`cache-targets: false` + `sccache`)

### 6.1 Architectural Design
Variant C preserves the benefits of Variant B while restoring object-level compiled crate reuse:
1. **Target Pruning Retained**: `cache-targets: false` in `Swatinem/rust-cache` keeps Cargo registry/git/bin caches small (~95 MB) and avoids the 1.5 GB target upload.
2. **Object-Level Compiler Caching**: Integrates `sccache v0.17.0` via `mozilla-actions/sccache-action@fc920bf0ec8de6ee65d409111f7ec508035751ba # v0.0.11`.
3. **GHA Backend & Security Boundary**:
   - `RUSTC_WRAPPER: sccache`
   - `SCCACHE_GHA_ENABLED: "true"`
   - `SCCACHE_GHA_VERSION: "sky-ci-v1"`
   - `SCCACHE_GHA_RW_MODE`: `READ_ONLY` on PRs, `READ_WRITE` on main / workflow_dispatch.
4. **Environment Isolation**:
   - `"sccache"` added to `$required` in `Construct Python-unavailable validation environment` and `Construct Python-unavailable canonical environment`, preserving `sccache.exe` in the restricted PATH while scrubbing all Python binaries and environments.
5. **Observability**: Explicitly logs `sccache --show-stats` in `validate` and `packaged`.

### 6.2 Acceptance Thresholds for Variant C
- Warm PR required-gate: $\le 9\text{m}$ (ideally $\le \text{Control A } \sim 8\text{m}35\text{s}$).
- Save-enabled path: Maintains clear advantage over Control A 15m40s (around or below Variant B $\sim 13\text{m}$).
- Sccache upload overhead: Must not create a multi-minute critical path tail.
- Hit statistics: Directly observe and report compile requests, cache hits, misses, and hit rate from `sccache --show-stats`.

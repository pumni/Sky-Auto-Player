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

| Metric | Main Baseline (#549)<br/>True Wall-Clock | PR Control (#548)<br/>True Wall-Clock | CI-FAST-1 (Hosted PR #96)<br/>Run `33665614053` | CI-FAST-2<br/>(Static / Supply-Chain) | CI-FAST-3<br/>(Cache Architecture) | Final Wave 8 |
| :--- | ---: | ---: | :---: | :---: | :---: | :---: |
| `changes` (PR) | 59s | 33s | **17s** (Run #551: **15s**) | — | — | TBD |
| `changes` (Dispatch/Main) | 59s | N/A | **3s** (Run #552) | — | — | TBD |
| `static` | 50s | 43s | **46s** (Run #551: **43s**) | target <= 35–40s | — | TBD |
| `supply_chain` | (in static) | (in static) | (in static) | parallel job | — | TBD |
| `validate` (PR restore-only) | N/A | 8m41s | **7m53s** (Run #551: 8m44s) | — | TBD | TBD |
| `validate` (Main with save) | 15m17s | N/A | N/A | — | TBD | TBD |
| `packaged` (PR restore-only) | N/A | 8m06s | **7m22s** (Run #551: 8m04s) | — | TBD | TBD |
| `packaged` (Main with save) | 10m33s | N/A | N/A | — | TBD | TBD |
| `status` (Required gate) | ~9s | ~5s | **3s** (Run #551: **5s**) | <= 5s | <= 5s | TBD |
| **Workflow Wall-Clock (PR)** | N/A | **9m31s** | **8m24s** | TBD | TBD | TBD |

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
   - `static` (`Static and security gates`): Pure repository static invariants (`cargo xtask check static --skip-supply-chain`). Completely offline, zero network access, no `cargo-audit` advisory DB downloading, no `cargo-vet` tool setup/restore.
   - `supply_chain` (`Supply-chain and advisory security`): Dedicated job on `ubuntu-latest` running in parallel with `static`. Restores and caches pinned `cargo-audit` (v0.22.2) and `cargo-vet` (v0.10.2). Executes `cargo audit --file rust/Cargo.lock` and `cargo vet --manifest-path rust/Cargo.toml --locked`.
2. **Offline Local Contract Preservation**:
   - Running `cargo xtask check static` locally without flags continues to execute all checks including `supply_chain::run(None)` by default.
   - Flag `--skip-supply-chain` or environment variable `SKY_CHECK_SKIP_SUPPLY_CHAIN=1` safely bypasses the redundant `cargo vet` step when executed under dedicated CI pipelines.
   - Preserves all security invariants from `SECURITY.md` and zero-Python audit rules from `AGENTS.md`.
3. **Gate Convergence**:
   - Updated `status` required CI gate to depend on `[changes, static, supply_chain, validate, packaged]`, failing closed if either `static` or `supply_chain` fails.

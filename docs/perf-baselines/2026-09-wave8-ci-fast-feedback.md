# Wave 8 — CI Fast-Feedback Performance Baseline & Report

Date recorded: 2026-09-03
Baseline commit: `main@6a6155df83294d0defe35f6ef4f774e2405c5a45` (PR #95 merge)
First post-#95 main workflow: `CI run #549 / 33659228830`

## 1. Post-#95 Authoritative Baseline

The baseline is the exact post-merge Wave 7 / PR #95 main qualification run (`33659228830`).

### 1.1 GitHub Authoritative Job & Workflow Timestamps

> [!IMPORTANT]
> GitHub Actions post-job steps (such as cache saves) run after job scripts complete and are on the critical path of workflow completion. Total job wall-clock includes post-actions. Below, timings are partitioned into **True Job Wall-Clock** (GitHub job lifecycle) and **Pre-Post / Productive Step Duration** (excluding post-actions).

| Job / Component | True Job Wall-Clock | Productive / Pre-Post | Key Phase Durations & Breakdown |
| :--- | ---: | ---: | :--- |
| **`changes` total** | **59s** | 59s | Checkout ~21s, rustup minimal ~7–8s, compile un-cached `sky_xtask` ~20s |
| **`static` total** | **50s** | 50s | Ubuntu runner, zero `rust-cache`, compiles `xtask` again |
| **`validate` total** | **15m17s** | ~7m30s | Rust-cache restore: **~60s** (~1.5 GB); Pre-post: ~7m30s; **Post Rust cache save: 6m27s** (17:18:19–17:24:46Z) |
| **`packaged` total** | **10m33s** | ~6m45s | Rust-cache restore: **~32s** (~842 MB); Pre-post: ~6m45s; **Post Rust cache save: 2m40s** (17:17:24–17:20:04Z) |
| **`status` (Required gate)** | **~9s** | ~9s | Convergence check after `validate` completes |
| **Total Workflow Wall-Clock** | **~16m38s** | — | Workflow start: `17:08:28Z` $\rightarrow$ status complete: `17:25:05Z` |

### 1.2 Critical Observations on Cache Economics
1. **The Post-Action Save Tax**: On `main` pushes (where `save-if: true` applies), saving the Rust cache archives consumes **6m27s** on `validate` and **2m40s** on `packaged`. In run #549, `validate` finished its productive tests at 17:18:19Z, but the job did not complete until 17:24:52Z solely due to cache compression and upload.
2. **Developer Feedback Latency**: Because the required gate (`status`) waits on all jobs, the true wall-clock time until green feedback on `main` was **16m38s**, not the pre-post estimate of ~8m30s.
3. **Input to CI-FAST-3**: This post-job save bottleneck confirms the hypothesis for CI-FAST-3: target-cache restoration and save economics must be evaluated against clean/sccache alternatives.

### 1.3 Engineering Targets

```text
ordinary PR p50 <= 5m
ordinary PR p95 <= 8m
main exact p50 <= 10m
```

These targets are performance goals; they must never be achieved by removing validation coverage or compromising security.

---

## 2. Wave 8 Execution Matrix & Phased Comparison

| Metric | Post-#95 Baseline (`6a6155df`)<br/>True Job Wall-Clock | Post-#95 Baseline<br/>Productive Step | CI-FAST-1 (Hosted PR #96)<br/>Run `33665614053` | CI-FAST-2<br/>(Static / Supply-Chain) | CI-FAST-3<br/>(Cache Architecture) | Final Wave 8 |
| :--- | ---: | ---: | :---: | :---: | :---: | :---: |
| `changes` (PR) | 59s | 59s | **17s** (compile: 1.12s) | — | — | TBD |
| `changes` (Main / dispatch) | 59s | 59s | target <= 2s (no checkout) | — | — | TBD |
| `static` (or `static-source`) | 50s | 50s | **46s** | target <= 35–40s | — | TBD |
| `supply_chain` | (in static) | (in static) | (in static) | parallel job | — | TBD |
| `validate` (total wall-clock) | 15m17s | ~7m30s | **7m53s** | — | TBD | TBD |
| `validate` Rust-cache restore | ~60s | ~60s | **51s** | — | TBD | TBD |
| `validate` Post Rust cache | 6m27s | — | **0s** (skipped on PR) | — | TBD | TBD |
| `packaged` (total wall-clock) | 10m33s | ~6m45s | **7m22s** | — | TBD | TBD |
| `packaged` Rust-cache restore | ~32s | ~32s | **26s** | — | TBD | TBD |
| `packaged` Post Rust cache | 2m40s | — | **0s** (skipped on PR) | — | TBD | TBD |
| `status` (Required CI gate) | ~9s | ~9s | **3s** | <= 5s | <= 5s | TBD |
| **Total Workflow Wall-Clock** | **16m38s** | — | **8m24s** | TBD | TBD | TBD |

---

## 3. CI-FAST-1 Hosted Validation Evidence

### 3.1 Candidate Execution Metadata
- **Pull Request**: [#96](https://github.com/pumni/Sky-Auto-Player/pull/96)
- **Candidate Branch**: `perf/ci-fast-1-bootstrap`
- **Candidate HEAD SHA**: `1ca1ada4abb97a4cbefcb737dd78717e225db161`
- **Workflow Run**: [CI Run #33665614053](https://github.com/pumni/Sky-Auto-Player/actions/runs/33665614053)
- **Workflow Lifecycle**: Started `2026-09-02T18:10:42Z` $\rightarrow$ Completed `2026-09-02T18:19:06Z` (Total Wall-Clock: **8m24s**)
- **Final Result**: **100% PASS** (all required gates green)
- **Adoption Status**: `ADOPTED` (Candidate verified on hosted GitHub Actions runners)

### 3.2 Individual Job Breakdown & Authoritative Timestamps

#### Job 1: `changes` (`Classify validation layers`)
- **Job ID**: `100366483302`
- **Runner**: `ubuntu-latest` (`GitHub Actions 1000003282`)
- **Lifecycle**: `18:10:46Z` $\rightarrow$ `18:11:03Z` (**17s** wall-clock)
- **Step Durations**:
  - Set up job: 1s
  - Start CI timing: <1s
  - Emit full qualification matrix: skipped (PR trigger)
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
- **Evaluation against target**: Job wall-clock **17s** meets target $\le 20\text{s}$. Classifier compilation was **1.12s**, resolving the previous ~20s `xtask` bootstrap tax.

#### Job 2: `static` (`Static and security gates`)
- **Job ID**: `100366597794`
- **Runner**: `ubuntu-latest` (`GitHub Actions 1000003285`)
- **Lifecycle**: `18:11:06Z` $\rightarrow$ `18:11:52Z` (**46s** wall-clock)
- **Step Durations**:
  - Checkout (`fetch-depth: 1`): 2s
  - Fetch Wave-6 retirement baseline (dynamic resolution from ledger JSON + `git cat-file -e`): 1s
  - Install toolchain: 8s
  - Restore cargo-audit (`actions/cache/restore@v6.1.0`): 2s
  - Restore cargo-vet (`actions/cache/restore@v6.1.0`): 2s
  - Cargo audit & Cargo vet: 7s
  - Repository verification — static: 18s
  - Report CI timing (native Bash): <1s
- **Conclusion**: `success`

#### Job 3: `validate` (`Windows compatibility and unit tests`)
- **Job ID**: `100366597848`
- **Runner**: `windows-latest` (`GitHub Actions 1000003283`)
- **Lifecycle**: `18:11:06Z` $\rightarrow$ `18:18:59Z` (**7m53s** wall-clock)
- **Step Durations**:
  - Checkout (`fetch-depth: 1`): 6s
  - Fetch Wave-6 retirement baseline (dynamic PowerShell from ledger + `git cat-file -e`): 1s
  - Install & verify stable Rust: 13s
  - Rust cache restore (`Swatinem/rust-cache`): 51s
  - Restore & verify cargo-vet (`actions/cache/restore@v6.1.0`): 2s
  - Bun setup: 1s
  - Construct Python-unavailable validation environment: 2s
  - Restricted repository verification without Python: 6m29s
  - Post Rust cache: <1s (save skipped on PR)
- **Conclusion**: `success`

#### Job 4: `packaged` (`Packaged frozen unsigned app smoke`)
- **Job ID**: `100366597914`
- **Runner**: `windows-latest` (`GitHub Actions 1000003284`)
- **Lifecycle**: `18:11:05Z` $\rightarrow$ `18:18:27Z` (**7m22s** wall-clock)
- **Step Durations**:
  - Checkout (`fetch-depth: 1`): 7s
  - Install & verify Rust toolchain: 11s
  - Rust cache restore: 26s
  - Bun setup: 2s
  - Construct Python-unavailable canonical environment: 1s
  - Build and qualify exact portable artifact (`cargo xtask dist --profile dist`): 6m28s
  - Verify packaged release tree: 1s
  - Upload exact portable release candidate (`actions/upload-artifact@v7.0.1`): 2s
  - Post Rust cache: <1s (save skipped on PR)
- **Artifact Verified**:
  - Name: `sky-auto-player-portable-1ca1ada4abb97a4cbefcb737dd78717e225db161`
  - Artifact ID: `9860715494`
  - File Size: `9,172,026 bytes` (107 files)
  - SHA-256 Digest: `fcd8d29ed4f570d5700feaa260b024eaec039222eb4ecaecfb2ef5e910893a4b`
  - URL: `https://github.com/pumni/Sky-Auto-Player/actions/runs/33665614053/artifacts/9860715494`
- **Conclusion**: `success`

#### Job 5: `status` (`Sky Auto Player — required CI gate`)
- **Job ID**: `100369273840`
- **Runner**: `ubuntu-latest` (`GitHub Actions 1000003286`)
- **Lifecycle**: `18:19:03Z` $\rightarrow$ `18:19:06Z` (**3s** wall-clock)
- **Gate Evaluation**: All upstream jobs succeeded (`changes: success`, `static: success`, `validate: success`, `packaged: success`).
- **Conclusion**: `success`

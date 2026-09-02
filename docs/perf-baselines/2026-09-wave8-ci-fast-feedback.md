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

| Metric | Post-#95 Baseline (`6a6155df`)<br/>True Job Wall-Clock | Post-#95 Baseline<br/>Productive Step | CI-FAST-1 (Candidate)<br/>Hosted PR Run | CI-FAST-2<br/>(Static / Supply-Chain) | CI-FAST-3<br/>(Cache Architecture) | Final Wave 8 |
| :--- | ---: | ---: | :---: | :---: | :---: | :---: |
| `changes` (PR) | 59s | 59s | *pending hosted run* | — | — | TBD |
| `changes` (Main / dispatch) | 59s | 59s | *pending hosted run* | — | — | TBD |
| `static` (or `static-source`) | 50s | 50s | *pending hosted run* | target <= 35–40s | — | TBD |
| `supply_chain` | (in static) | (in static) | — | parallel job | — | TBD |
| `validate` (total wall-clock) | 15m17s | ~7m30s | *pending hosted run* | — | TBD | TBD |
| `validate` Rust-cache restore | ~60s | ~60s | *pending hosted run* | — | TBD | TBD |
| `validate` Post Rust cache | 6m27s | — | *restore-only on PR* | — | TBD | TBD |
| `packaged` (total wall-clock) | 10m33s | ~6m45s | *pending hosted run* | — | TBD | TBD |
| `packaged` Rust-cache restore | ~32s | ~32s | *pending hosted run* | — | TBD | TBD |
| `packaged` Post Rust cache | 2m40s | — | *restore-only on PR* | — | TBD | TBD |
| `status` (Required CI gate) | ~9s | ~9s | *pending hosted run* | <= 5s | <= 5s | TBD |
| **Total Workflow Wall-Clock** | **16m38s** | — | *pending hosted run* | TBD | TBD | TBD |

---

## 3. Candidate CI-FAST-1 — Eliminate Classifier / Bootstrap Tax

### 3.1 Hypothesis
1. Replace monolithic `sky_xtask` invocation in `changes` with a dedicated, zero-dependency Rust crate `sky_ci_classifier` (`std`-only).
2. Short-circuit `--full` on `push` / `workflow_dispatch` without checking out git or running `rustup`/Cargo.
3. Replace full-history `fetch-depth: 0` with shallow `fetch-depth: 1` + targeted base commit fetch on PR.
4. Eliminate PowerShell timing bootstrap overhead on Ubuntu runners (use native Bash `date +%s`).
5. Read historical Wave-6 baseline SHA directly from `docs/migration/wave6-tooling-retirement-ledger.json` instead of hard-coding.
6. Refine concurrency to isolate `workflow_dispatch` manual qualifications from automatic branch cancellation.
7. Upgrade pinned GitHub Actions dependencies (reconcile PR #94: `actions/cache` v6.1.0, `upload-artifact` v7.0.1, `action-gh-release` v3.0.3).

### 3.2 Candidate Specification & Implementation
- Candidate branch: `perf/ci-fast-1-bootstrap`
- Candidate commits:
  - `perf(ci): add zero-dependency Rust path classifier`
  - `perf(ci): eliminate classifier bootstrap tax, shallow checkout, and reconcile action pins`
  - `docs(perf): record Wave 8 fast-bootstrap baseline and plan`
  - *corrections commit pending*

### 3.3 Hosted CI Verification Evidence
- Candidate PR: *to be opened upon push*
- Workflow Run ID: *pending*
- Candidate Status: `IN_PROGRESS`
*(Status will be updated to ADOPTED only after all hosted gates pass and exact hosted timestamps/evidence are recorded.)*

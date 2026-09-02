# Wave 8 — CI Fast-Feedback Performance Baseline & Report

Date recorded: 2026-09-03
Baseline commit: `main@6a6155df83294d0defe35f6ef4f774e2405c5a45` (PR #95 merge)
First post-#95 main workflow: `CI run #549 / 33659228830`

## 1. Post-#95 Authoritative Baseline

The baseline is the post-merge Wave 7 / PR #95 main qualification run (`33659228830`).

### 1.1 Observed Workflow & Job Timings

| Job / Component | Duration | Notes / Cache State |
| :--- | ---: | :--- |
| **`changes` total** | **~59s** | Full clone + rustup minimal + compile un-cached `sky_xtask` |
| `changes / checkout` | ~21s | `fetch-depth: 0` full git history clone |
| `changes / rustup` | ~7–8s | Minimal toolchain install (no rustup cache) |
| `changes / compile+run classifier` | ~20s | Compiles `sky_xtask` + 10 crates with zero cache |
| **`static` total** | **~50s** | Ubuntu runner, zero `rust-cache`, compiles `xtask` again |
| **`validate` total** | **~7m30s** | Windows runner |
| `validate / Rust cache restore` | ~60s | Restores ~1.5 GB cache archive |
| **`packaged` total** | **~6m45s** | Windows runner |
| `packaged / Rust cache restore` | ~32s | Restores ~842 MB cache archive |
| **Workflow wall-clock** | **~8m30s** | Total critical path on merged main |

### 1.2 Engineering Targets

```text
ordinary PR p50 <= 5m
ordinary PR p95 <= 8m
main exact p50 <= 10m
```

These targets represent performance goals; they must never be achieved by removing validation coverage or compromising security.

---

## 2. Wave 8 Execution Matrix & Phased Comparison

| Metric | Post-#95 baseline (`6a6155df`) | CI-FAST-1 (Bootstrap) | CI-FAST-2 (Static & Supply-Chain) | CI-FAST-3 (Cache Architecture) | Final Wave 8 |
| :--- | ---: | ---: | ---: | ---: | ---: |
| `changes` (PR) | ~59s | target <= 15–20s | — | — | TBD |
| `changes` (Main / dispatch) | ~59s | target <= 5s | — | — | TBD |
| `static-source` | ~50s (combined static) | — | target <= 35–40s | — | TBD |
| `supply_chain` | (combined in static) | — | parallel job | — | TBD |
| `validate` | ~7m30s | — | — | TBD | TBD |
| `packaged` | ~6m45s | — | — | TBD | TBD |
| `validate / Rust cache restore` | ~60s | ~60s | ~60s | TBD (B/C experiment) | TBD |
| `packaged / Rust cache restore` | ~32s | ~32s | ~32s | TBD (B/C experiment) | TBD |
| `status` (Required CI gate) | ~10s | <= 5s | <= 5s | <= 5s | TBD |
| Full critical path (PR) | ~8m30s | TBD | TBD | TBD | TBD |

---

## 3. Candidate CI-FAST-1 — Eliminate Classifier / Bootstrap Tax

### 3.1 Hypothesis
1. Replace monolithic `sky_xtask` invocation in `changes` with a dedicated, zero-dependency Rust crate `sky_ci_classifier` (`std`-only).
2. Short-circuit `--full` on `push` / `workflow_dispatch` without checking out git or running `rustup`/Cargo.
3. Replace full-history `fetch-depth: 0` with shallow `fetch-depth: 1` + targeted base commit fetch on PR.
4. Eliminate PowerShell timing bootstrap overhead on Ubuntu runners.
5. Upgrade pinned GitHub Actions dependencies (reconcile PR #94).

### 3.2 Adoption Status
- Status: `IN_PROGRESS`

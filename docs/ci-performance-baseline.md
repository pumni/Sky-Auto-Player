# CI performance baseline and measurement method

This is a revision-bound observation, not a CI speedup claim. The historical
Actions measurements below were collected from public run/job/step metadata
with `scripts/ci_metrics.mjs`. The tool is read-only: it calls `gh api`, does
not download logs, and emits normalized JSON plus Markdown. The captured data
is retained in
[`evidence/ci-performance-baseline-20261112.json`](evidence/ci-performance-baseline-20261112.json).

## Historical hosted runs

| Run | Category | Source SHA | Attempt | Result | Wall time | Sum of runner time | Jobs |
| --- | --- | --- | ---: | --- | ---: | ---: | ---: |
| [43053012918](https://github.com/pumni/Sky-Auto-Player/actions/runs/43053012918) | UI CI | `dd9e61202178d1f00a4c065230d9ecf12c4136c8` | 1 | success | 383 s | 7.7667 min | 15 |
| [43053013206](https://github.com/pumni/Sky-Auto-Player/actions/runs/43053013206) | UI hosted proof | `dd9e61202178d1f00a4c065230d9ecf12c4136c8` | 2 | failure: hosted ClamAV DNS/bootstrap | 495 s | 15.15 min | 2 |
| [43027304308](https://github.com/pumni/Sky-Auto-Player/actions/runs/43027304308) | Full CI | `885473a36c13e49557a0e666f6d2a18ad2072332` | 1 | success | 912 s | 49.6333 min | 15 |
| [43027305170](https://github.com/pumni/Sky-Auto-Player/actions/runs/43027305170) | Full hosted proof | `885473a36c13e49557a0e666f6d2a18ad2072332` | 1 | success | 621 s | 11.6833 min | 2 |
| [43101297576](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576) | WP-A classifier/workflow change | `eaa199be2cdbb46cbb0ebe15f20d475807298036` | 1 | failure: Windows Clippy | 691 s | 50.0333 min | 17 |

The UI-only and full-validation rows are different commits and workload
categories. The ClamAV DNS failure is retained as a failed proof sample. The
WP-A run changed the classifier and CI workflow, so it selected full validation;
it is **not** evidence for a UI-only PR workload. Do not subtract the rows and
claim a measured CI optimization from these observations.

At the exact WP-A source SHA, the following hosted jobs succeeded:

- [Independent unsigned-before-sign test-key proof](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576/job/130793372492).
- [Build current Tauri candidate](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576/job/130793372545).
- [ClamAV positive-control malware proof](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576/job/130793372715).
- [Packaged NSIS qualification](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576/job/130796799254).
- [Updater fixture qualification](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576/job/130797469321), including installing the supplied bridge and packaged updater rotation.

The [Windows compatibility and unit tests job](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576/job/130793372418)
failed its Rust verification step because Clippy rejected two overwritten
`type_only` assignments. Consequently, the [required CI gate](https://github.com/pumni/Sky-Auto-Player/actions/runs/43101297576/job/130798741838)
failed. Hosted proof success does not convert that required status into a pass.

The current malware gate uses hosted Ubuntu ClamAV and its EICAR positive
control. This does not establish Windows Defender cloud or behavioral
detection. Security-owner approval of that residual limitation remains an
explicit pre-merge decision on the PR; this implementation makes no claim of
equivalence and does not replace the malware proof.

## Interpretation and limits

- Each sample has a distinct category, event/workflow, source revision, and
  conclusion. Each category has `n=1`, so the report intentionally has no p50
  or p95.
- Runner minutes sum measured job durations, including failed jobs with valid
  timestamps and excluding skipped/cancelled nodes. Parallel jobs make this sum
  differ from workflow wall time. It is runner occupancy, not GitHub billable
  minutes or OS-weighted billing units.
- The Actions run/job REST responses used here do not expose a per-job runner
  OS. The report marks OS as `unknown`; it does not infer Windows or Ubuntu from
  job names. OS subtotals are therefore unavailable for these samples.
- Workflow completion uses the run endpoint's `updated_at` when the run is
  complete. Queue time is created-to-started. The observed longest job is not a
  formal dependency-DAG critical path because the collected API data does not
  include the workflow dependency graph.
- Job/step records with missing timestamps remain `null`. Skipped/cancelled
  nodes are excluded even when Actions returns inconsistent timestamps. Failed
  jobs are classified from their conclusion only; inspect the linked job logs
  to determine root cause.
- sccache summaries are not included in the Actions metadata response. The tool
  does not download job logs, so those values are explicitly unavailable.
- Runner scheduling, network/DNS, cache state, hosted image updates, and
  concurrent workload affect durations. Compare only the same category and
  workflow/event/conclusion over enough repetitions.

## Local Windows Rust benchmark

The reproducible cold/warm Windows compile and test measurements are recorded
below after running the prescribed commands on an isolated `CARGO_TARGET_DIR`.
The benchmark source SHA/tree is the implementation commit named in the results;
the later evidence/documentation commit does not change the measured Rust or
CI-tooling source.

### Measurement procedure

On Windows PowerShell, set a new, user-owned target directory under `$env:TEMP`
and keep it for every repetition. Do not remove or reuse `rust/target` or an
existing Rust cache. Record Windows/CPU details, `rustc -Vv`, `cargo -V`, the
source SHA/tree, `RUSTC_WRAPPER`, `CARGO_BUILD_RUSTC_WRAPPER`, and
`CARGO_INCREMENTAL`. If `sccache` is present, capture `sccache --show-stats`
before and after. The benchmark commands are:

```powershell
cargo test --manifest-path rust/Cargo.toml --workspace --all-features --locked --no-run
cargo test --manifest-path rust/Cargo.toml --workspace --all-features --locked -- --test-threads=1
```

Run each once against the empty target and then repeat as a warm run; execute at
least two warm test runs if time permits. The first command compiles test
targets. The later test command can still rebuild some objects, so compilation
and execution are command-level phases, not guaranteed independent work.

<!-- WP-B local benchmark measurements are inserted here before handoff. -->

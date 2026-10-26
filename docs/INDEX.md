# Sky Auto Player Documentation Router

Choose the smallest set of documents that matches the task. Evidence and decision records are
revision-scoped; their presence here or in the repository does not make them current instructions.

## Find documents by task

- **Architecture and native ownership:** [architecture.md](architecture.md),
  [native-playback-ownership.md](native-playback-ownership.md), and
  [rt-dispatch-architecture.md](rt-dispatch-architecture.md).
- **Playback and timing:** [timing-principles.md](timing-principles.md),
  [hold-frame-model.md](hold-frame-model.md), and [tuning-presets.md](tuning-presets.md).
- **Timing or Windows qualification:** [final-down-admission-qualification.md](final-down-admission-qualification.md)
  and [physical-key-study-method.md](physical-key-study-method.md). The associated
  [timing observations](rt-dispatch-timing-evidence.md) are revision-scoped evidence.
- **Desktop UI and workbench layout:** [desktop-workbench.md](desktop-workbench.md).
- **Library and local import:** [library-v1-contract.md](library-v1-contract.md).
- **Security and distribution:** [../SECURITY.md](../SECURITY.md),
  [distribution-and-update.md](distribution-and-update.md),
  [v4-release-authority.md](v4-release-authority.md),
  [v4-tauri-packaging.md](v4-tauri-packaging.md),
  [v4-updater-key-custody.md](v4-updater-key-custody.md),
  [v4-authenticode-provider-seam.md](v4-authenticode-provider-seam.md), and
  [v4-release-execution-topology.md](v4-release-execution-topology.md), and
  [v4-hosted-signing-proof.md](v4-hosted-signing-proof.md).
- **CI lanes and artifact tooling:** [ci-validation-topology.md](ci-validation-topology.md)
  covers Rust-owned classification, candidate/bridge artifacts, calibration packaging, and
  installer/updater qualification. [powershell-dependency-ledger.md](powershell-dependency-ledger.md)
  records migrated fixture entrypoints and retained Windows/operator adapters.
- **Toolchain and development:** [rust-toolchain-policy.md](rust-toolchain-policy.md) and
  [../CONTRIBUTING.md](../CONTRIBUTING.md).
- **Performance or test-pipeline work:** [desktop-startup-benchmark.md](desktop-startup-benchmark.md)
  documents an opt-in benchmark; [maintenance-test-pipeline-report.md](maintenance-test-pipeline-report.md)
  records measured gate coverage and costs. [Dispatch forensics](dispatch-forensics-cost-qualification.md)
  and the [whole-stream timing advisory](prepare-whole-stream-timing-advisory.md) are
  source-revision-scoped research, not general current behavior contracts.

## Evidence and authority

Authority depends on the claim being checked; there is no single source hierarchy for every domain.

- Current implementation behavior is established by source, direct tests, and configuration at the
  exact checked-out SHA. CI reports which checks passed for its own commit.
- Security and release requirements come from the current security and distribution contracts,
  considered with the enforcing source and workflows.
- Claims about Windows behavior, physical input, or observed playback require evidence from the
  matching native environment. Browser screenshots and mock bridges establish only the UI behavior
  they exercise.
- Historical decisions and captures establish only what their stated revision, date, and conditions
  support. They are not standing instructions or evidence of a current approval state.

Use [adr/](adr/) for the relevant design decision and its rationale; ADRs are not all current
contracts and need not be read as a set. Use [evidence/](evidence/) for revision-bound measurements,
screenshots, and reports, and [releases/](releases/) for qualification records tied to a particular
release. Check each record's source SHA, conditions, and stated limits before applying it.

## Verification

The canonical pre-review command is `cargo xtask check all`; it covers static, Rust, desktop, and
native desktop checks. Focused development commands are `cargo xtask check static`,
`cargo xtask check rust`, and `cargo xtask check desktop`. Packaging, Windows timing acceptance,
release, and benchmark workflows are specialized evidence paths for changes that touch those
boundaries.

# Technical Documentation — Find by Task

This is a locator, not a checklist. Open the references relevant to the
problem; code and tests at the checked-out SHA establish implementation
behavior. Security, timing, and release contracts also define intended
invariants that must not be discarded because an implementation differs.

## Active contracts and references

- **Architecture and ownership:** [architecture](architecture.md);
  [native dispatch](rt-dispatch-architecture.md) for real-time boundaries.
- **Timing and playback:** [timing principles](timing-principles.md),
  [hold frames](hold-frame-model.md), [presets](tuning-presets.md);
  [timing observation semantics](rt-dispatch-timing-evidence.md)
  (field definitions are current; dated qualification results are historical).
- **Desktop UI:** [workbench](desktop-workbench.md).
  **Library:** [playlist/import contract](library-v1-contract.md).
- **Security:** [SECURITY.md](../SECURITY.md).
- **Song source schemas and canonical decoding:** [song source decoding](song-source-decoding.md).
- **Canonical events to Sky Piano plans:** [song planning](song-planning.md).
- **Packaging and updater:** [distribution](distribution-and-update.md),
  [Tauri packaging](v4-tauri-packaging.md).
- **Release operations:** [release authority](v4-release-authority.md),
  [execution topology](v4-release-execution-topology.md)
  (active runbook; dated v4.2.0 compatibility proof is historical),
  [updater key custody](v4-updater-key-custody.md),
  [Authenticode](v4-authenticode-provider-seam.md).
- **CI and development:** [CI topology](ci-validation-topology.md),
  [Rust toolchain](rust-toolchain-policy.md),
  [contributing](../CONTRIBUTING.md).
- **When measuring or investigating:** [startup benchmark](desktop-startup-benchmark.md),
  [physical-key study method](physical-key-study-method.md),
  [whole-stream timing advisory](prepare-whole-stream-timing-advisory.md).
- **When auditing agent context:** [maintenance approach](agent-context-maintenance.md)
  and [optional behavioral probes](agent-context-evals.md).

## Revision-bound records

These are useful when tracing a decision or reproducing an earlier result,
not as instructions for current code. Keep recorded SHAs, hardware,
measurements, and limitations intact.

- **Decisions:** [ADRs](adr/).
- **Measured / reviewed evidence:** [evidence](evidence/),
  [native playback ownership migration](native-playback-ownership.md),
  [hosted signing proof](v4-hosted-signing-proof.md),
  [dispatch forensics qualification](dispatch-forensics-cost-qualification.md),
  [final Down qualification](final-down-admission-qualification.md),
  [test-pipeline report](maintenance-test-pipeline-report.md),
  [PowerShell migration ledger](powershell-dependency-ledger.md), and
  [Issue #560 Package D qualification](evidence/issue-560-package-d/README.md)
  (source-bound 96-song paging/Windows profile; not current 106-song or installed-upgrade evidence).
- **Release-specific records:** [releases](releases/).
- Other issue- and phase-scoped reports remain in `docs/` (for example,
  `gate-*-evidence.md`, `r3-*.md`, and `prepared-*-evidence.md`);
  search by issue, feature, or source SHA when needed.

If an active contract disagrees with source, inspect both and report the
discrepancy before changing the relevant owner. Historical test evidence
and browser mocks cannot establish current native Windows timing behavior.

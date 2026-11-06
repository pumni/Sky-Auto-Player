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
- **Desktop UI:** [workbench](desktop-workbench.md),
  [current UI image reference](ui/README.md) when visual inspection is needed,
  [React Aria type boundary](desktop-type-boundary.md),
  [desktop JavaScript tooling](desktop-tooling.md).
  **Library:** [playlist/import contract](library-v1-contract.md).
- **Sheet compatibility and playback consent:** [desktop workbench](desktop-workbench.md#sheet-compatibility-and-playback-preparation),
  [source decoding](song-source-decoding.md), [15-key planning](song-planning.md),
  and [numeric compatibility](song-numeric-compatibility.md) (maintained rules;
  recorded migration inventory is revision-bound).
- **Built-in songs:** [manifest](../builtin-songs/manifest.json) for current entries;
  [song curation audit](song-curation/README.md) for review methods and
  revision-bound inventory evidence.
- **Security:** [SECURITY.md](../SECURITY.md).
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
- **Archived qualification and issue evidence:** [historical archive index](HISTORY.md)
  (original reports, run logs, binary traces and screenshot provenance are
  retrievable at a pinned main commit; not current qualification).
- **Retained reproduction datasets:** [native test parity baseline](evidence/issue-540-native-playback-test-parity.json)
  and [wake-calibration checksum-backed projection](evidence/wake-calibration-qualification-20261018.json).
- **Legacy PowerShell audit:** [migration ledger](powershell-dependency-ledger.md)
  (dated migration record; current release/CI owners are the Rust sources).
- **Release-specific records:** [releases](releases/).
- Use [HISTORY.md](HISTORY.md) to retrieve removed issue/phase reports by full
  source SHA; do not mistake those archived results for current behavior.

If an active contract disagrees with source, inspect both and report the
discrepancy before changing the relevant owner. Historical test evidence
and browser mocks cannot establish current native Windows timing behavior.

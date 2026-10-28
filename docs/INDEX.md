# Sky Auto Player — Documentation Router

Use this page to identify the smallest document set needed for the current
task. It is a **router**, not a required reading list. Current behavior is
determined by code, direct tests, and configuration at the checked-out SHA;
written contracts and runbooks add the project's intended invariants.

## Current contracts and task-specific references

| Task or boundary | Read when relevant | Authority and scope |
| --- | --- | --- |
| Application layers and module ownership | [architecture.md](architecture.md) | Current architectural boundaries; compare against checked-out source |
| Native scheduling, focus, sender, and cleanup | [rt-dispatch-architecture.md](rt-dispatch-architecture.md), [timing-principles.md](timing-principles.md) | Normative dispatch invariants; verify Windows behavior on native hardware |
| Musical hold/release floors and user presets | [hold-frame-model.md](hold-frame-model.md), [tuning-presets.md](tuning-presets.md) | Timing policy and supported presets |
| Playback diagnostics or timing observations | [rt-dispatch-timing-evidence.md](rt-dispatch-timing-evidence.md) | Observation semantics and limitations, not proof of game-side audio |
| Desktop React layout, responsive behavior | [desktop-workbench.md](desktop-workbench.md) | Current workbench behavior; browser emulation is not native geometry validation |
| Library import and playlists | [library-v1-contract.md](library-v1-contract.md) | Library data and navigation contract |
| Security or trust boundary | [../SECURITY.md](../SECURITY.md), [architecture.md](architecture.md) | Security rules together with enforcing code and tests |
| Installer, updater, and release behavior | [distribution-and-update.md](distribution-and-update.md), [v4-release-authority.md](v4-release-authority.md) | Current distribution contract and release runbook |
| Protected release runner and candidate lifecycle | [v4-release-execution-topology.md](v4-release-execution-topology.md) | Current release execution topology |
| Update key handling or Authenticode | [v4-updater-key-custody.md](v4-updater-key-custody.md), [v4-authenticode-provider-seam.md](v4-authenticode-provider-seam.md) | Operational security contracts; never paste secrets into context |
| Tauri package assembly | [v4-tauri-packaging.md](v4-tauri-packaging.md) | Packaging reference; confirm current flags and assets in source |
| CI classification and repository tooling | [ci-validation-topology.md](ci-validation-topology.md), [rust-toolchain-policy.md](rust-toolchain-policy.md), [../CONTRIBUTING.md](../CONTRIBUTING.md) | Current validation topology, pinned tools, and contributor workflow |
| Measuring desktop startup | [desktop-startup-benchmark.md](desktop-startup-benchmark.md) | Opt-in measurement procedure, not a standing benchmark requirement |
| Playback timing study or physical-key experiment | [physical-key-study-method.md](physical-key-study-method.md) | Research method; evidence is revision-bound |
| Whole-stream timing estimates | [prepare-whole-stream-timing-advisory.md](prepare-whole-stream-timing-advisory.md) | Advisory/model; not proof of game input sampling |
| Agent guidance or doc cleanup | [agent-context-maintenance.md](agent-context-maintenance.md), [agent-context-evals.md](agent-context-evals.md) | Agent-maintenance method and optional behavioral test cases |

## Historical records — look up only when the task needs provenance

The following sources are **snapshots**, not instructions to reinstate an old
architecture or claims that current HEAD passed historic qualification:

- [native-playback-ownership.md](native-playback-ownership.md): before/after
  extraction and historical parity record. Current ownership: [architecture.md](architecture.md).
- [v4-hosted-signing-proof.md](v4-hosted-signing-proof.md): test-key/hosted-signing
  migration proof. Current release authority: [v4-release-authority.md](v4-release-authority.md)
  and [v4-release-execution-topology.md](v4-release-execution-topology.md).
- [powershell-dependency-ledger.md](powershell-dependency-ledger.md):
  migration dependency ledger; inspect actual scripts/workflows for current callers.
- [Graphite final polish evidence](evidence/graphite-final-ui-polish/README.md):
  revision-bound UI acceptance captures, not a native Windows geometry guarantee.
- [maintenance-test-pipeline-report.md](maintenance-test-pipeline-report.md),
  [dispatch-forensics-cost-qualification.md](dispatch-forensics-cost-qualification.md),
  [final-down-admission-qualification.md](final-down-admission-qualification.md):
  bounded validation/measurement records.
- [gate-a-312-evidence.md](gate-a-312-evidence.md),
  [gate-b-312-evidence.md](gate-b-312-evidence.md),
  [phase-7-301-evidence.md](phase-7-301-evidence.md),
  [post-adr-0017-hardening-final-evidence.md](post-adr-0017-hardening-final-evidence.md),
  [prepared-causal-admission-final-evidence.md](prepared-causal-admission-final-evidence.md),
  [prepared-dispatch-300-evidence.md](prepared-dispatch-300-evidence.md),
  [r3-keyboard-diagnostics.md](r3-keyboard-diagnostics.md),
  [r3-keyboard-final-evidence.md](r3-keyboard-final-evidence.md), and
  [r3-keyboard-p0-baseline.md](r3-keyboard-p0-baseline.md):
  issue-, phase-, or revision-specific records.

Additional historical sources are grouped in
[adr/](adr/), [evidence/](evidence/), and [releases/](releases/).
An ADR preserves a decision at its date; it is not automatically the active
contract. Evidence retains its stated hardware, SHA, methodology, and limits.
See [README.md](README.md) for documentation lifecycle.

## Resolving disagreements

- Current implementation claims: read the source, direct tests, and relevant
  configuration at the checked-out SHA; report documentation drift rather
  than treating older prose as executable truth.
- Intended security, dispatch, and release requirements: use the responsible
  current contract **and** enforcing checks; an implementation mismatch is
  a finding to resolve, not permission to discard a safety requirement.
- Historic performance or physical behavior: require matching native
  measurements and source identity. Browser mocks, fixtures, and prior-release
  evidence cannot establish current Windows timing behavior.
- If two current documents conflict, identify both precise passages and the
  owner of the boundary before editing. Avoid speculative reconciliation.

## Verification

Run focused checks while editing. The canonical pre-review gate is
`cargo xtask check all` once on the final relevant source state. Specialized
package, updater, release, security, and Windows timing acceptance apply when
the affected boundary requires them. Record checks actually run and any gaps.

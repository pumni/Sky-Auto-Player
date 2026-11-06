# Archived checkpoint reports — immutable source revision

The following **revision-specific** reports were removed from the active `docs/` tree to keep task navigation focused. Their complete original text, SHA identities, test environment, and caveats remain in the immutable Git commit [`69cdb8598a2a`](https://github.com/pumni/Sky-Auto-Player/tree/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs). No new measurements or requalification are implied.

| Archived report | What it records |
| --- | --- |
| [Issue #312 Gate A](https://github.com/pumni/Sky-Auto-Player/blob/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs/gate-a-312-evidence.md) | structural hot-path cleanup checkpoint |
| [Issue #312 Gate B](https://github.com/pumni/Sky-Auto-Player/blob/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs/gate-b-312-evidence.md) | prepared-dispatch benchmark and dead-window cleanup |
| [Issue #300 prepared-dispatch](https://github.com/pumni/Sky-Auto-Player/blob/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs/prepared-dispatch-300-evidence.md) | suspension reconciliation and measurements |
| [R3 P0 baseline](https://github.com/pumni/Sky-Auto-Player/blob/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs/r3-keyboard-p0-baseline.md) | native keyboard characterization only |
| [R3 P4 diagnostics](https://github.com/pumni/Sky-Auto-Player/blob/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs/r3-keyboard-diagnostics.md) | HOL/startup and offline receiver evidence |
| [R3 P5 final checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs/r3-keyboard-final-evidence.md) | historical evidence; explicitly **not qualified for merge** |
| [Post-ADR-0017 P8/H4](https://github.com/pumni/Sky-Auto-Player/blob/69cdb8598a2a60c6a6854c1cafe4546dea83fdbe/docs/post-adr-0017-hardening-final-evidence.md) | dated coordinator-review-pending qualification |

These are **not** instructions or proof about current HEAD. For live implementation contracts start with [INDEX.md](INDEX.md), source, and tests. Release notes (`docs/releases/`), ADRs (`docs/adr/`), and security/timing/release owner contracts remain in-tree; they were not deleted or rewritten in this cleanup.

The Git history URLs are deliberately commit-pinned, not branch-head URLs; do not relocate these historical reports back into current documentation unless a concrete reproduction or audit requires it.

# Technical Documentation — Find by Task

Open only the contract relevant to the change. Code and tests at the checked-out
SHA describe implementation; active security, timing, and release contracts
describe intended invariants. This index is a route map, not a reading checklist.

## Current contracts

- **Ownership and module boundaries:** [architecture](architecture.md).
- **Native playback:** [engine and timing](playback-engine.md) for scheduling,
  admission, focus, and cleanup; [verification](playback-verification.md) for
  native sink and timing qualification. Browser mocks are not native timing evidence.
- **Desktop UI:** [workbench](desktop-workbench.md) for user interactions,
  [React Aria types](desktop-type-boundary.md), and
  [JavaScript tooling](desktop-tooling.md). Consult the
  [current visual reference](ui/README.md) only for visual work; check its capture SHA.
- **Library and built-in songs:** [library identity/membership](library-v1-contract.md),
  [bundled manifest](../builtin-songs/manifest.json), and
  [catalog curation/qualification](song-curation/README.md).
- **Sheet compatibility:** [source decoding](song-source-decoding.md),
  [15-key planning](song-planning.md), and the
  [workbench preparation flow](desktop-workbench.md#sheet-compatibility-and-playback-preparation).
- **Security:** [security boundary](../SECURITY.md).
- **Packaging and updates:** [distribution](distribution-and-update.md) and
  [Tauri packaging](v4-tauri-packaging.md).
- **Release or signing:** [release authority](v4-release-authority.md),
  [execution/recovery](v4-release-execution-topology.md),
  [updater key custody](v4-updater-key-custody.md), and
  [Authenticode](v4-authenticode-provider-seam.md).
- **CI and development:** [CI checks](ci-validation-topology.md),
  [Rust toolchain](rust-toolchain-policy.md), and
  [contributing](../CONTRIBUTING.md).

## History is not a live contract

- [HISTORY.md](HISTORY.md) links original decisions, snapshots, and evidence
  at immutable Git SHAs. Use them for provenance, not current qualification.
- Published user-facing notes live in [GitHub Releases](https://github.com/pumni/Sky-Auto-Player/releases).
  An unreleased candidate still needs source-tagged `docs/releases/{tag}.md`
  before preflight; see [release authority](v4-release-authority.md).

If an active contract and the checked-out source disagree, investigate both
before changing the owning source, tests, or contract. Report uncertainty
rather than silently treating old implementation details as permanent.

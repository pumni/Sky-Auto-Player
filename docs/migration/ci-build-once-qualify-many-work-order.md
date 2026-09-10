# CI Build-Once / Qualify-Many — Codex Work Order

## Goal

Refactor the ordinary CI data plane so package qualification and updater qualification consume the same exact current-candidate bytes instead of independently building the current candidate.

This is PR2 after the merged control-plane refactor. Keep the four-workflow architecture and the existing classifier taxonomy unchanged unless a narrowly required contract update is needed for this data-plane change.

Core invariant:

`ONE SOURCE SHA -> ONE CURRENT CANDIDATE BUILD -> many tests consume the same exact bytes -> ONE QUALIFIED BYTE SET`

Production release/rehearsal remain separate and continue to own their production/rehearsal attestations and release authority.

## Scope boundary

This PR is about current-candidate artifact identity and fan-out only.

Do not implement PR3 latency work:

- do not move browser/frontend validation from Windows to Ubuntu
- do not split the current validate job into new runner topology
- do not perform broad cache redesign
- do not add merge queue
- do not weaken the Rust `profile.dist`
- do not introduce a reusable release workflow abstraction
- do not redesign the path classifier or its risk matrix
- do not prebuild the updater bridge unless it is strictly required to preserve the existing updater contract; the bridge may continue to build once inside updater qualification

Keep the required check name exactly:

`Sky Auto Player — required CI gate`

## Current problem

Today the package lane builds the canonical current Tauri candidate itself, while the updater lane invokes the updater fixture without a provided current candidate and therefore builds a separate current candidate during updater qualification.

The updater scripts already expose the required provided-candidate interface:

- `CandidateInstallerPath`
- `CandidateSignaturePath`
- `CandidateVersion`
- `CandidatePublicKeyPath`

Use that interface rather than designing a parallel updater harness.

## Required job topology

Add one ordinary-CI current-candidate producer job in `.github/workflows/ci.yml`.

Recommended job identity: `candidate` / `Build current Tauri candidate`.

Run it when:

`package_required == true || updater_required == true`

The candidate producer must:

1. check out the exact PR head / main SHA already selected by CI;
2. install the pinned Rust/Bun/tooling needed to build the current Tauri candidate;
3. build the desktop frontend once;
4. generate one bounded ephemeral Tauri updater signing keypair for this CI candidate;
5. build the current Tauri NSIS candidate exactly once with `--profile dist` and the existing unsigned-zero-budget Authenticode policy;
6. verify that exactly one current installer and exactly one matching Tauri updater signature exist;
7. compute and record exact SHA-256 digests;
8. upload one workflow artifact containing the current candidate contract described below;
9. delete the private updater test key before artifact upload / job completion and never include it in outputs, summaries, artifacts, or logs.

Do not perform install/uninstall qualification or updater E2E in the producer. It is a producer, not another qualification lane.

## Candidate artifact contract

Upload one artifact tied to the exact source SHA, for example:

`sky-auto-player-current-candidate-${source_sha}`

Its extracted payload must contain exactly the current-candidate inputs needed by downstream qualification:

- the single NSIS installer
- the matching official Tauri updater `.sig`
- the corresponding test updater public key only
- `candidate.json`

The private updater key is forbidden from the artifact.

`candidate.json` must be small and deterministic enough for downstream validation. At minimum record:

- schema version
- exact `source_sha`
- canonical application `version`
- installer filename
- installer SHA-256
- updater-signature filename
- updater-signature SHA-256
- updater-public-key filename
- updater-public-key SHA-256

Use repository-relative/artifact-relative filenames, not runner-local absolute paths.

Downstream jobs must re-hash all referenced files after download and fail closed on any filename, SHA, version, source-SHA, multiplicity, or path mismatch.

Do not trust artifact names alone as identity.

## Package qualification consumer

Change the existing `packaged` lane from producer+consumer to consumer only.

It must depend on the candidate job and download the exact artifact produced by that same workflow run.

After download it must:

- validate `candidate.json` against the current CI source SHA and source version;
- recompute installer/signature/public-key hashes before using the files;
- use the downloaded installer and signature as the only current candidate under package qualification;
- preserve the existing package-specific checks that actually validate the candidate bytes, including unsigned-zero-budget Authenticode verification, SBOM generation/verification as applicable to the exact downloaded installer, exact-bundle identity checks, built-in catalog checks, current-user install/launch/uninstall, installed-tree verification, and qualification evidence;
- ensure qualification evidence refers to the downloaded candidate hashes, not to a newly built artifact.

The package job must not run `tauri build` for the current candidate.

If existing xtask commands assume `rust/target/dist/bundle/nsis`, stage/copy the downloaded exact bytes into a bounded expected directory without modifying the bytes, or minimally extend the command to accept the downloaded location. Prefer the smaller change that preserves existing verification semantics.

## Updater qualification consumer

Change `updater_e2e` to depend on the candidate producer and download the exact same artifact from the same workflow run.

Validate `candidate.json` and re-hash the exact files before updater testing.

Invoke the existing updater harness with the provided-candidate contract:

- `-CandidateInstallerPath <downloaded exact installer>`
- `-CandidateSignaturePath <downloaded exact signature>`
- `-CandidateVersion <candidate.json version>`
- `-CandidatePublicKeyPath <downloaded public key>`

The updater lane must not rebuild the current candidate.

The previous-version bridge may still be built once inside updater qualification. Do not broaden this PR into bridge artifact caching/prebuild work.

## Preserve old-root rejection without rebuilding current candidate

Keep the updater key-rotation safety property.

For the old-root rejection branch:

- use the exact downloaded current-candidate installer bytes;
- copy those bytes to a disposable path if needed for fixture isolation;
- sign that copy with the fixture old updater root to create an old-root signature;
- serve a synthetic higher-SemVer manifest that references those same candidate bytes with the old-root signature;
- exercise the real updater download/verification path with the new-only client and require rejection;
- never rebuild a second "current" candidate solely for this negative test.

The byte identity of the installer used for new-root acceptance and old-root rejection must be demonstrably the same before signing metadata/signature differences are applied.

Preserve existing user-data/built-in-catalog preservation checks and loopback HTTP evidence.

## Authenticode/signing contract placement

Ordinary package qualification should focus on the exact downloaded candidate.

Move contract-only Authenticode regression/signing tests out of the package smoke critical path and into the existing Windows release/signing contract lane where practical:

- `scripts/test_v4_authenticode_integrity.ps1`
- `scripts/test_v4_production_signing_contract.ps1`

These tests must not require building another application installer. A bounded disposable PE fixture/system PE copy is acceptable for the Authenticode integrity contract.

Keep candidate-specific unsigned-zero-budget verification in package qualification.

Do not weaken production signing fail-closed behavior.

## Ordinary CI attestations

Remove ordinary-CI GitHub artifact attestation creation/verification from the package lane after the build-once artifact flow is established.

Specifically, ordinary CI does not need to create provenance attestations for every package-sensitive main/manual run merely to pass package smoke.

Preserve:

- exact SHA-256 identity in the candidate contract and qualification evidence
- workflow-artifact transfer within the same CI run
- production release attestations
- controlled rehearsal attestations
- production SBOM/provenance/release security invariants

After removing ordinary CI attestations, remove now-unneeded `id-token: write` / `attestations: write` permissions from ordinary CI jobs. Keep only permissions actually required by their remaining actions.

## Artifact and trust rules

- Use the artifact from the current workflow run only; do not search previous runs by name.
- Pin any new GitHub Action by full commit SHA, consistent with repository policy.
- The candidate artifact is transport, not authority. Downstream consumers must validate source SHA, version, names and hashes from `candidate.json`.
- No private updater/signing key may cross a job boundary.
- Do not persist runner-local absolute paths in candidate metadata.
- Do not mutate candidate installer bytes between producer upload and consumer qualification.

## Required CI aggregate behavior

Update `Sky Auto Player — required CI gate` dependencies/expected result logic for the new candidate job.

Expected semantics:

- if neither package nor updater is required, candidate is skipped;
- if package or updater is required, candidate must succeed;
- package and updater consumers may run in parallel after candidate succeeds;
- package-only diffs build one candidate and run package qualification only;
- updater-only diffs build one candidate and run updater qualification only;
- package+updater diffs build one candidate and fan the same bytes to both consumers.

Do not create another required branch-protection status.

## Contract/self-test updates

Update the existing repository static contracts in `rust/xtask/src/checks.rs` and PowerShell release/CI contract tests so they enforce the new architecture rather than just matching renamed steps.

At minimum add fail-closed contract coverage that proves:

- candidate job is required for `package || updater`;
- both package and updater jobs depend on candidate;
- package job does not contain the current-candidate `tauri build`;
- updater job passes all four provided-candidate parameters;
- candidate artifact metadata includes exact source/version/hash identity;
- candidate private key is not uploaded;
- ordinary CI attestation steps/permissions are gone;
- production/rehearsal attestation markers remain present;
- aggregate required gate accounts for candidate success/skipped state.

Where practical, make candidate metadata validation a small reusable script with direct self-tests rather than duplicating fragile inline parsing in multiple jobs. Do not create a framework; one focused PowerShell helper plus tests is enough if needed.

## Acceptance cases

The implementation is accepted only if all of these hold:

1. A package+updater-sensitive PR builds the current candidate exactly once.
2. Both downstream lanes download the same current-run artifact and verify the same installer SHA-256.
3. Package qualification performs no second current `tauri build`.
4. Updater qualification performs no second current `tauri build`.
5. Updater bridge construction remains bounded to the updater lane and does not become another current-candidate producer.
6. New-root updater acceptance uses the exact candidate artifact.
7. Old-root rejection uses the same installer bytes and rejects the old-root signature through the real updater download/verification path.
8. Candidate artifact contains no private key or secret signing material.
9. Package smoke still covers install/launch/uninstall and candidate-specific Authenticode/bundle integrity.
10. Authenticode/signing contract tests remain covered outside the package critical path.
11. Ordinary CI attestation creation/verification is removed; production/rehearsal attestations are unchanged.
12. Required gate name remains exactly `Sky Auto Player — required CI gate`.
13. Existing classifier taxonomy and four-workflow control plane remain unchanged.
14. No PR3 runner/cache/latency redesign is introduced.
15. Full CI is green on this PR, including package and updater lanes consuming the shared candidate.

## Validation expectations

Because this PR modifies `.github/workflows/ci.yml` and CI contracts, it should classify itself as full validation.

Before merge, require green evidence for:

- classifier/static contracts
- Rust/native validation
- desktop/browser validation
- supply chain
- release/signing contract lane including moved Authenticode contracts
- current-candidate producer
- package consumer
- updater consumer including old-root rejection
- site validation
- aggregate required gate

During review, inspect the workflow logs/artifacts to prove one current build and matching SHA-256 identity across both consumers; do not accept job names alone as evidence.

## Follow-up after this PR

Only after this data-plane PR is green and merged should PR3 optimize critical-path latency: move frontend/browser coverage to Ubuntu, split native/frontend validation where measured, benchmark caches/tool setup, and consider prebuilding the updater bridge only if measurements justify it.

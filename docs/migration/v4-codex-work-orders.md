# V4 Codex Work Orders

Date: 2026-09-03

This file is the execution queue for heavy implementation work intended to run locally on Windows.
The governing architecture is `docs/adr/ADR-0006-v4-distribution-installation-update.md`; the execution
specification is `docs/migration/v4-clean-distribution-execution.md`.

## Global instructions for every work order

Codex should treat the ADR as authoritative. If current code conflicts with it, report the conflict and
implement toward the ADR rather than preserving legacy behavior by default.

For every work order:

1. Start from current `main`, not from another unfinished work-order branch unless the dependency says
   otherwise.
2. Create a focused branch named `v4/<work-order-slug>`.
3. Read the relevant production code, tests, ADRs, and release tooling before editing.
4. Do not modify historical v3 tags/releases/evidence.
5. Do not introduce arbitrary network endpoints, updater URLs, signing bypasses, downgrade switches,
   or test-only seams into production configuration.
6. Keep React behind bounded Rust commands/events for update authority.
7. Prefer deleting legacy code only after the replacement path has packaged acceptance coverage.
8. Run the narrowest useful checks while iterating, then run `cargo xtask check all` before opening the
   PR unless the work order explicitly documents an environmental blocker.
9. Run `git diff --check` before committing.
10. In the PR body include: architecture changes, files removed, tests run, packaged/manual checks still
    outstanding, and any ADR deviation. Do not silently change the ADR.

Do not combine multiple work orders into one large PR unless a compile-time dependency makes separation
impossible. When that happens, preserve the acceptance criteria of both work orders and explain why.

---

## WO-01 — Tauri NSIS + updater packaging foundation

Dependency: ADR-0006 accepted.

Suggested branch: `v4/tauri-packaging-foundation`

### Objective

Make Tauri the canonical v4 Windows package producer without yet deleting the v3 updater implementation.
Produce a per-user NSIS installer and Tauri updater artifacts using non-production/test signing material
for local qualification.

### Required changes

- Update the Tauri configuration for the permanent v4 identifier selected by ADR-0006.
- Enable bundle generation and target NSIS only for the canonical v4.0 Windows package.
- Enable Tauri updater artifacts.
- Add the official updater/process plugin dependencies at compatible pinned versions.
- Establish one canonical SemVer version source and update version-lock checks accordingly.
- Keep current-user/no-admin NSIS behavior explicit or covered by a static assertion/test.
- Extend `xtask` or a focused verifier to discover and validate the generated installer/updater artifact
  set rather than assuming the v3 portable ZIP is the only package form.
- Add documentation for the local Windows build command and expected output files.

### Do not

- remove `sky_updater` yet;
- switch runtime update checks to Tauri updater yet;
- commit production signing private keys;
- publish v4 artifacts into the v3 GitHub Releases namespace;
- preserve PEP 440 compatibility in the new v4 package version source unless required solely to keep
  unreleased v3 maintenance code compiling.

### Acceptance

- Windows local build produces an NSIS installer.
- Installer installs for current user without requiring Administrator privileges.
- Tauri updater artifact/signature outputs are produced with test signing material.
- Installed app launches and packaged GUI smoke passes.
- Existing v3 production updater code remains buildable until its replacement is complete.
- `cargo xtask check all` passes or the PR records a narrowly scoped pre-existing/environmental failure.

---

## WO-02 — Clean v4 application-data boundary

Dependency: WO-01 may proceed in parallel if file conflicts are kept small.

Suggested branch: `v4/app-data-boundary`

### Objective

Remove the architectural reason for an updater preserve list by ensuring the v4 install root contains
only installer-owned application payload.

### Required changes

- Inventory every production read/write relative to the executable/install root.
- Move mutable settings, logs, cache, calibration state, and database/index state to OS-appropriate
  app-data locations owned by the v4 application identity.
- Remove production `.env`-beside-executable behavior. Keep development-only environment overrides only
  when they are explicitly scoped and safe.
- Ensure music/library files remain user-owned sources or application-managed data outside the install
  root.
- Introduce one Rust path service/adapter as the authority for these directories rather than scattering
  path construction across modules.
- Add deterministic tests for path ownership and for a read-only/replaceable install root.
- Update packaged smoke tests to run with the install payload treated as immutable.

### Optional, separate boundary

A v3 importer may be scaffolded only as an explicit user action. It may read selected legacy settings
and song locations but must not make v4 depend on the v3 updater or v3 install layout at normal startup.

### Acceptance

- Normal v4 runtime performs no mutable writes beside the executable.
- Uninstall/reinstall of the application payload can preserve v4 user data.
- No v4 update/install logic requires `config.json`, `.env`, `songs/**`, or `logs/**` preserve-list
  semantics.
- Tests enumerate and enforce installer-owned versus application/user-owned paths.

---

## WO-03 — Rust-owned Tauri UpdateService

Dependency: WO-01.

Suggested branch: `v4/tauri-update-service`

### Objective

Replace the v4 runtime update path with a narrow Rust application service around the official Tauri
updater while leaving `sky_updater` available only as legacy code until WO-06.

### Required changes

- Add a Rust `UpdateService` boundary under the desktop/runtime application layer.
- Model bounded states/DTOs for current, checking, available, downloading, ready/installing, and error.
- Own stable/beta endpoint selection in compiled Rust configuration. React must not provide URLs.
- Implement update check through the official Tauri updater.
- Implement download/install/relaunch flow with bounded progress reporting.
- Reject installation while playback is active.
- Add the updater pre-exit safety boundary: stop playback, quiesce the worker, release all injected keys,
  persist required state, and close bounded resources before installation exits the application.
- Keep update notes bounded and treat remote text as untrusted display content.
- Reject downgrade by default.
- Add unit tests for update policy and packaged/integration tests using a local/static fixture endpoint or
  other framework-supported deterministic test setup that cannot be enabled in production.

### Do not

- expose `@tauri-apps/plugin-updater` as the authority directly from React;
- accept caller-supplied endpoints/artifact paths;
- add signature-verification bypasses;
- remove legacy updater files in this PR.

### Acceptance

- React invokes only bounded application commands/events.
- A deterministic previous-v4 fixture can discover and apply a candidate v4 update in packaged testing.
- Playback-active install is rejected.
- Pre-exit input cleanup is covered by automated packaged evidence.
- No production v4 update flow launches `Sky-Auto-Player-Updater.exe`.

---

## WO-04 — Dedicated v4 release metadata and channel authority

Dependency: operational creation of the dedicated v4 release authority; WO-01 for artifact names.

Suggested branch: `v4/release-authority`

### Objective

Define and validate the v4 binary release namespace and static Tauri updater metadata so v3 and v4
cannot discover each other's releases.

### Required changes

- Add repository configuration/docs for the approved v4 release-authority repository/namespace.
- Define canonical NSIS/updater artifact names from actual Tauri build output; do not invent duplicate
  aliases unless a consuming tool requires them.
- Generate static updater metadata for stable and, if enabled, beta.
- Make metadata promotion a distinct post-qualification action.
- Add CI/static tests proving runtime endpoint allow-listing and channel isolation.
- Add an explicit guard that release workflows cannot upload v4 canonical artifacts to the v3 release
  namespace by mistake.
- Add an acceptance check that the v3 source repository `/releases/latest` response remains a v3 release
  when v4 candidate artifacts exist in the separate authority.

### Acceptance

- A v3 client cannot see a v4 release through its existing discovery endpoint.
- A stable v4 client cannot consume beta metadata.
- Update metadata references only already-published/qualified v4 assets.
- Metadata generation is deterministic from qualified release inputs.

---

## WO-05 — Authenticode, updater-key operations, SBOM, and attestations

Dependency: WO-01 and selected signing provider/credentials. Parts without credentials may be prepared
first, but real signing acceptance requires the configured provider.

Suggested branch: `v4/release-trust-chain`

### Objective

Establish the v4 Windows publisher identity and independent updater trust root, then make both mandatory
release gates.

### Required changes

- Integrate the selected Authenticode signing provider through Tauri/CI-supported configuration.
- Sign every project-owned PE that ships in the canonical installer and sign the installer itself as
  required by the packaging flow.
- Verify Authenticode signatures after signing and again during exact-artifact qualification.
- Generate a new v4 Tauri updater key pair/trust root; never reuse the v3 `release-2026` key.
- Keep private material out of the repository and logs.
- Document updater-key backup, loss, compromise, and rotation procedures.
- Build a non-production key-rotation fixture/test proving a client can transition according to the
  chosen rotation mechanism.
- Generate a standard SPDX JSON or CycloneDX JSON SBOM.
- Preserve cargo-vet policy evidence.
- Generate GitHub build provenance and SBOM attestations and verify them in qualification.

### Acceptance

- Release qualification fails closed when canonical artifacts are unsigned or signatures do not verify.
- No private signing material appears in repository files or CI logs.
- SBOM and provenance correspond to the exact candidate artifacts.
- Test-key rotation is repeatable and documented.

---

## WO-06 — Replace v3 release assembly and retire custom updater from v4

Dependency: WO-01 through WO-05 complete and packaged acceptance green.

Suggested branch: `v4/retire-legacy-updater`

### Objective

Remove the v3 custom updater and portable release contract from the production v4 dependency/release
path only after the standard replacement is proven.

### Required changes

- Remove `sky_updater` from the production desktop dependency graph.
- Stop building/packaging `Sky-Auto-Player-Updater.exe` for v4.
- Remove native update handoff/launch paths that exist only for the custom updater.
- Remove v4 release ZIP assembly, custom manifest signature publication, and v3 updater transaction
  qualification from the v4 release workflow.
- Remove PEP 440 ordering from the v4 update/release path.
- Reframe `cargo xtask dist`/release commands around Tauri-produced artifacts or retire obsolete commands
  with explicit replacement commands.
- Preserve historical v3 documentation/evidence; update current normative docs so users do not confuse
  v3 and v4 distribution models.
- Add static audits that fail if the canonical v4 runtime/release tree regains dependencies on retired
  updater artifacts or v3 update state directories.

### Acceptance

- Canonical v4 build/release succeeds with no `sky_updater` production dependency.
- Canonical v4 package contains no `Sky-Auto-Player-Updater.exe`.
- Canonical v4 release contains no v3 custom update `MANIFEST.json.sig` contract unless a separately
  reviewed non-updater use requires it.
- Fresh install and previous-v4 update acceptance remain green.
- Historical v3 tags/releases remain untouched.

---

## WO-07 — Exact v4 release workflow and GA qualification

Dependency: WO-01 through WO-06.

Suggested branch: `v4/release-pipeline`

### Objective

Make the release workflow implement the ADR state machine: build once, sign, attest, draft, qualify the
exact assets, publish immutably, then promote updater metadata.

### Required changes

- Build canonical v4 artifacts once from the exact tag/commit.
- Verify version/tag/SemVer lock before packaging.
- Sign and verify artifacts.
- Generate/verify SBOM, cargo-vet evidence, and provenance.
- Create a draft release in the dedicated v4 release authority.
- Download/qualify the exact draft assets rather than rebuilding locally.
- Run the full Windows qualification matrix.
- Publish the already-qualified draft as the immutable release.
- Promote stable/beta static updater metadata only after publish succeeds.
- Verify published asset identities/digests and immutable state.
- Make any failed qualification require a new version/RC.

### Full qualification matrix

- fresh current-user install;
- no-admin installation semantics;
- launch and packaged GUI smoke;
- playback/input safety smoke;
- previous-v4 -> candidate-v4 update;
- update-install rejection during playback;
- uninstall;
- reinstall with existing user data;
- Authenticode verification;
- updater signature verification;
- SBOM/provenance verification;
- exact downloaded-installer Defender custom scan with no detection (missing
  Defender cmdlets, disabled protection, scan failure, or detection fails
  closed; unavailable evidence is not promotable);
- stable/beta isolation;
- v3/v4 namespace isolation;
- exact published asset digest match;
- immutable release check.

### Acceptance

- `v4.0.0-rc.1` can be produced using the exact same state machine intended for GA.
- No manual asset replacement or tag movement is required after draft creation.
- Stable metadata cannot point at an unqualified or draft release.

---

## Recommended execution order

```text
ADR-0006
   |
   +--> WO-01 packaging --------+
   |                            |
   +--> WO-02 app data          +--> WO-03 update service
                                |        |
                                |        +--> WO-04 release authority
                                |                 |
                                +---------------> WO-05 trust chain
                                                  |
                                                  v
                                           WO-06 retirement
                                                  |
                                                  v
                                           WO-07 release CI
```

WO-02 can run in parallel with much of WO-01. WO-04 has an external dependency: the dedicated v4
release authority must exist before production endpoint wiring. WO-05 has an external dependency:
real Authenticode acceptance needs the chosen signing provider and credentials.

## Stop conditions for Codex

Codex must stop the current work order and report instead of improvising when any of these occur:

- the implementation would publish v4 into the v3 release discovery namespace;
- the permanent application identifier or release authority is still ambiguous for a production edit;
- a signing secret/private key would need to be committed, echoed, or generated into a tracked path;
- satisfying the task appears to require a frontend-supplied updater URL/key/artifact path;
- tests reveal that stopping playback cannot guarantee all injected keys are released before updater
  exit;
- the task requires rewriting or deleting historical v3 release evidence;
- a Tauri API/config assumption differs from the currently pinned version and cannot be verified from
  the local lockfile/docs.

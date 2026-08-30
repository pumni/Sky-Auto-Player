# Phase 8 packaging evidence

Phase 8 produces one unsigned portable release candidate from the exact
checked-out commit. The authoritative generated evidence is emitted beside
the artifact by `scripts/build_phase8.py`:

- `PROVENANCE.json` records the public repository head, pinned Python/Rust/Bun/
  Tauri/PyInstaller identities, exact ZIP hash, manifest hash, and file count.
- `PHASE8_ARTIFACT_SUMMARY.json` repeats the final ZIP size/hash, MANIFEST hash,
  portable file count, and managed-entry count for CI summaries and review.
- `PHASE8_QUALIFICATION.json` records the Core self-test, packaged
  Tauri/Core pairing smoke, the fail-closed Core self-test negative matrix,
  packaged GUI smoke, packaged TUI smoke, and exact-artifact updater
  qualification.
- `MANIFEST.json` is copied from the assembled tree and the ZIP contains that
  same embedded manifest. The `.sha256` sidecar hashes the exact ZIP bytes.

The CI packaged job uploads the exact output directory without rebuilding the
ZIP after qualification. The release tree is:

```text
Sky-Auto-Player-v3.5.0/
├── Sky-Auto-Player.exe
├── Sky-Auto-Player-Core.exe
├── Sky-Auto-Player-Updater.exe
├── native_calibration.exe
├── MANIFEST.json
├── _internal/
└── songs/
```

The package smoke is run from a temporary Windows path containing spaces. It
uses no repository checkout, source `.venv`, Bun server, game process, or
physical input. The headless Tauri self-test uses the production release launch
command and `CoreSupervisor` to validate the packaged shell/Core pairing. A
second packaging-only smoke launches the real Wry window and lets the
production React store exercise bootstrap, Library search, settings
round-trip, diagnostics enable/disable, and the explicit no-input calibration
seam through the real Tauri bridge before the approved controlled shutdown
path closes the window. GUI smoke assertions are fail-closed and invalid child
output is captured as bytes. Exact GUI visual acceptance remains a manual
release check.

Previous-stable qualification is `3.4.5 → 3.5.0` and exercises the native
updater's ZIP/sidecar/manifest checks, staged verification, managed-file
transition, preserved user paths, transaction installation, injected apply
failure rollback, and interrupted transaction recovery against the actual
Phase 8 ZIP. The feature-gated
`sky_updater_e2e` runner supplies only the deterministic local-release
transport used by the offline exact-artifact handoff/restart harness; the
shipped `Sky-Auto-Player-Updater.exe` remains the default GitHub/HTTPS
production binary and is independently identity-checked from the assembled
artifact. The established updater corpus and fault-injection tests remain
separate required gates.

The offline harness cannot claim a public GitHub release transaction without
publishing a release or introducing a local HTTP/socket source. It therefore
records the production updater identity check separately from the exact ZIP
transaction/restart qualification rather than weakening the production
source/trust policy.

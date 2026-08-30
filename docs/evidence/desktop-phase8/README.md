# Phase 8 packaging evidence

Phase 8 produces one unsigned portable release candidate from the exact
checked-out commit. The authoritative generated evidence is emitted beside
the artifact by `scripts/build_phase8.py`:

- `PROVENANCE.json` records the public repository head, pinned Python/Rust/Bun/
  Tauri/PyInstaller identities, exact ZIP hash, manifest hash, and file count.
- `PHASE8_QUALIFICATION.json` records the Core self-test, packaged
  Tauri/Core pairing smoke, packaged TUI smoke, and exact-artifact updater
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
round-trip, and diagnostics enable/disable through the real Tauri bridge
before the approved controlled shutdown path closes the window. Exact GUI
visual acceptance remains a manual release check.

Previous-stable qualification is `3.4.5 → 3.5.0` and exercises the native
updater's ZIP/sidecar/manifest checks, staged verification, managed-file
transition, preserved user paths, and transaction installation against the
actual Phase 8 ZIP. The established updater corpus and fault/rollback tests
remain separate required gates.

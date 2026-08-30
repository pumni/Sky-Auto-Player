# Phase 7 update-flow qualification

Date: 2026-08-30

This is the Phase 7 update-flow qualification, not the Phase 8 final portable
artifact qualification. The public source revision for the desktop update
surface is `fc549b6`. Native/updater provenance is the checked-in
`rust/crates/sky_updater` source plus its test-only deterministic fixture; a
final updater binary is intentionally not asserted at this phase.

## Controlled previous-stable → candidate fixture

`rust/crates/sky_updater/tests/packaged_update_e2e.rs` models a previous stable
installation at `3.4.5` and a v4-candidate target layout at `3.5.0`. It runs
the existing verified install transaction and injected rollback path, with
these user-owned files seeded before the update:

- `config.json`
- `.env`
- `songs/user.skysheet`
- `logs/old.log`
- an unrelated user file

The test confirms managed files are replaced, managed orphans are removed,
the canonical `MANIFEST.json` is committed, and all seeded user-owned files
survive both the successful transaction and rollback recovery.

## Matrix

| Boundary | Evidence | Result |
| --- | --- | --- |
| Stable/beta policy and version selection | `tests/test_update_service.py` | PASS |
| Typed Core check/preferences/handoff dispatch | `tests/test_desktop_ipc.py` | PASS |
| Active install guard: live, stale/dead, canonical identity | `tests/test_active_update_guard.py` | PASS |
| Verified Python handoff staging and bounded ready wait | `tests/test_update_launcher.py` | PASS |
| HTTPS/asset/manifest/sidecar/path validation | `sky_updater` unit tests | PASS |
| Previous stable → candidate transaction | `packaged_update_e2e.rs` | PASS |
| Fault injection and rollback | `packaged_update_e2e.rs`, `updater_safety.rs` | PASS |
| User-file preservation | `packaged_update_e2e.rs` | PASS |
| Canonical restart target | native updater restart tests and contract docs | PASS |

The native updater remains the only download, artifact-verification,
transaction, rollback, and canonical restart authority. React sends only a
typed target-version intent through DesktopBridge; it does not download a
release, inspect hashes as authority, invoke a path, or launch a process.

## Phase boundary

The qualification deliberately does not claim the final Tauri portable
package, PyInstaller sidecar cutover, `bundle.active: true`, or a production
release artifact. Those checks belong to Phase 8.

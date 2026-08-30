# Phase 7 update-flow qualification

Date: 2026-08-30

This is the Phase 7 update-flow qualification, not the Phase 8 final portable
artifact qualification. The prior public source revision for the desktop
update surface was `2cc36cb`; the updater-lifecycle closure is recorded in the
PR #76 verification section at the final branch head. Native/updater
provenance is the checked-in `rust/crates/sky_updater` source plus its
test-only deterministic fixture; a final updater binary is intentionally not
asserted at this phase.

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

## Desktop handoff lifecycle closure

The desktop launch path performs a read-only active-update admission check
before constructing the Tauri builder. It reuses `sky_updater`'s fixed
`active-update.json` path, install-ID, run-directory, and bounded Windows
process-image validation. A live owned transaction refuses ordinary GUI/Core
startup; foreign, stale, dead, and malformed state follows the existing
cleanup/ignore semantics without an indefinite wait.

The authoritative parent chain is:

```text
Tauri GUI PID
  -> Core --parent-pid
  -> UpdateLaunchRequest.parent_pid
  -> native updater --parent-pid
```

Legacy Textual launchers pass their own PID explicitly. The Core refuses a
desktop handoff without a positive bounded parent PID. A native launcher result
of `already_running` becomes a typed `update_busy` response and never emits
`update.handoff_ready`.

Only a successful typed handoff is followed by the Tauri `shutdown` command.
That command enters the shared `prevent-close`/idempotent close transition,
performs bounded Core/native cleanup on a blocking worker, and destroys the
window only after cleanup. Repeated close requests are ignored after the first
transition. Focused tests cover parent PID propagation, startup admission,
already-running handoff rejection, generated Tauri shutdown dispatch, and
cleanup-before-destroy ordering.

The native updater remains the only download, artifact-verification,
transaction, rollback, and canonical restart authority. React sends only a
typed target-version intent through DesktopBridge; it does not download a
release, inspect hashes as authority, invoke a path, or launch a process.

## Phase boundary

The qualification deliberately does not claim the final Tauri portable
package, PyInstaller sidecar cutover, `bundle.active: true`, or a production
release artifact. Those checks belong to Phase 8.

# Consolidation Performance Evidence

This record captures the measurements available while executing the native
consolidation at the 2026-09-02 Windows host. It is bounded evidence for this
worktree, not a universal benchmark.

## Tauri asset compression

Decision: RETAIN `tauri/compression`; the disable experiment is rejected as
inconclusive.

Method:

- same Windows host and frontend build;
- `dist` profile;
- separate clean `CARGO_TARGET_DIR` directories;
- compression ON used the `desktop-runtime` feature;
- compression OFF used the same runtime feature with only
  `tauri/compression` removed temporarily, then the source Cargo.toml was
  restored;
- no package or frontend source changes were made between variants.

| Variant | Clean build | Wall time | Shell EXE |
| --- | ---: | ---: | ---: |
| ON | 1 | 379.46 s | 10,316,800 bytes |
| OFF | 1 | 223.56 s | 10,424,320 bytes |
| OFF | 2 | 404.11 s | 10,424,320 bytes |

The OFF measurements vary by 180.55 seconds, so this host does not provide a
stable sample from which to claim a deterministic compile-time improvement.
The requested six clean builds (three per variant), exact ZIP comparison,
packaged GUI smoke, and startup comparison were not completed because each
clean Tauri release build took roughly four to seven minutes and the required
sample would not fit the execution window. No threshold-based adoption claim
is made. Compression remains enabled and the release feature graph is
unchanged except for the already-approved removal of unused Linux/runtime
features.

## Compiler cache experiment

Decision: do not add `sccache`.

The current repository keeps the existing workspace target cache. No
equivalent hosted-run sample exists in this execution context for the
required no-op, player edit, app-core edit, shell edit, dependency edit,
frontend-only edit, and cold-cache matrix. Adding `sccache` without those
measurements would stack cache mechanisms without evidence of lower critical
path time or cache bandwidth. The release and CI workflows therefore retain
the current cache architecture.

## CI cache and history choices

The pinned `cargo-audit` tool cache is restore-first and saves only on
non-pull-request runs after a miss. Browser setup is path-aware through the
xtask classifier, while the browser E2E gate remains present whenever a
browser-sensitive path changes.

The existing full-history checkout remains in jobs that use repository
classification, exact commit provenance, or release metadata. No checkout
optimization was adopted without proving that those inputs remain complete.

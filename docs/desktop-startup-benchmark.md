# Desktop startup telemetry and benchmark

The packaged desktop shell has an opt-in startup trace controlled by
`SKY_STARTUP_TELEMETRY_PATH`. When the variable is absent, native telemetry
does not open a file and the frontend does not issue telemetry IPC calls.
Telemetry is JSONL with a monotonic `elapsed_us` value relative to
`process.entry`. It contains marker names and bounded catalog counters only;
filesystem paths and imported source IDs are never written. Imported sources
are identified in the trace as `source_0`, `source_1`, and so on in manifest
order. Frontend markers also carry `frontend_elapsed_us`, captured from
`performance.now()` at the marker call site; frontend duration calculations use
that clock rather than the time when the native IPC request is received.

The startup milestones are intentionally separate:

- `catalog.cached_available` means a structurally valid private cache index has
  been published and is queryable before source reconciliation.
- `catalog.ready` means the native catalog index has been coherently replaced by
  a complete source composition.
- `catalog.reconciled` means background source reconciliation and cache
  persistence have completed.
- `react.shell_ready` means the React shell has rendered after native bootstrap and settings.
- `react.catalog_ready` means the initial catalog search and navigation reconciliation completed.

The initial bootstrap uses the settings snapshot loaded during native runtime
construction. `settings.reload.start/end` therefore belong to an explicit
settings refresh boundary after bootstrap, not to the initial shell bootstrap
critical path. The packaged smoke refresh occurs only after
`react.catalog_ready`, and the validator enforces that no settings reload is
observed before that milestone.

Catalog performance is reported as separate milestones: shell ready,
`cached_catalog_available_ms`, `background_reconciliation_complete_ms`, and
`cold_no_cache_rebuild_ms`. `cached_catalog_available_ms` measures the native
cache-to-queryable duration from `native.create.start`; the accompanying
`cached_catalog_available_at_ms` is the process-relative milestone timestamp.
Canonical paths remain native-only cache data and are never included in IPC or
telemetry.

Build and run the reproducible packaged benchmark from PowerShell:

```powershell
Set-Location desktop
bun install --frozen-lockfile
bun run build
bun run tauri build --ci -- --profile dist
bun run benchmark:startup -- `
  --exe ..\rust\target\dist\sky_desktop_shell.exe `
  --output ..\startup-baseline.json
```

The exact executable name can differ with the local Tauri target; pass the
packaged `.exe` with `--exe` when autodetection does not find it. The harness
creates all fixtures outside the repository and removes them after completion:

- minimal user library;
- a 12-file representative imported folder;
- a synthetic 40-directory × 32-file recursive tree.

Each fixture has a cold-ish set and a warm set. Both sets discard one warm-up
run and then execute 10 measured process restarts. Cold-ish means a fresh
app-data root and manifest; Windows filesystem/OS caches are not cleared.
Warm means one app-data root is reused after the discarded warm-up. The JSON
report contains the raw samples, median, p95, and the methodology used. Each
run fails validation unless the required Tauri/native/settings markers are
present, all elapsed clocks are monotonic, start/end pairs are ordered, and the
source marker set and counters match the fixture (including the 12-file and
1,280-file imported trees).

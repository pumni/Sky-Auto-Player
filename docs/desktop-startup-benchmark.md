# Desktop startup telemetry and benchmark

The packaged desktop shell has an opt-in startup trace controlled by
`SKY_STARTUP_TELEMETRY_PATH`. When the variable is absent, native telemetry
does not open a file and the frontend does not issue telemetry IPC calls.
Telemetry is JSONL with a monotonic `elapsed_us` value relative to
`process.entry`. It contains marker names and bounded catalog counters only;
filesystem paths and imported source IDs are never written. Imported sources
are identified in the trace as `source_0`, `source_1`, and so on in manifest
order.

The startup milestones are intentionally separate:

- `catalog.ready` means the native catalog index has been replaced and is queryable.
- `react.shell_ready` means the React shell has rendered after native bootstrap and settings.
- `react.catalog_ready` means the initial catalog search and navigation reconciliation completed.

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
report contains the raw samples, median, p95, and the methodology used.


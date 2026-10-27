# Native continuous resize qualification

This is the Wave A follow-up requested in
[PR #558](https://github.com/pumni/Sky-Auto-Player/pull/558#issuecomment-6445982924).
Results and compressed DevTools traces are in [native/README.md](native/README.md).

## Source and artifact identity

- Before: frontend bundle from `9d5818bc5ac7f26a1e29dd0a8a7e358cc0d7cd85`,
  entry `index-BjoSRmUi.js`.
- Intermediate inherited-property measurement:
  `6992f3d0b6d8d436595a96da53db99ce6455e6af`.
- Final runtime source: `2fec7501a2f4e7f863669246863f2918b33ab64d`,
  entry `index-BF9n3eZT.js`. Later evidence-only commits preserve this runtime source.
- [Before](native/before-artifact.json) and [final](native/final-artifact.json)
  record executable, JavaScript, instrumented HTML and measurement-preload SHA-256
  identities. The preload hash is identical. [Trace hashes](native/trace-manifest.json)
  cover every archived before/final trace and one representative intermediate trace.

The baseline native executable embedded the existing production frontend bundle
from the reviewed SHA; frontend source edits made during harness preparation were
not rebuilt into that frozen baseline bundle. The executable was copied to
`%TEMP%/sky-resize-before.exe` before rebuilding the changed frontend. Native Rust
source was unchanged. A valid baseline small-library batch was resumed with the
large-library batch using that same executable hash; JSON records both actual
measurement batches, not a synthesized run.

Both builds use production Vite/React bundles in a Windows **debug-profile Rust
shell with packaged-assets**, not a release performance qualification. No
shipping runtime, Tauri permissions, security contract or CI configuration was
modified for measurement. A measurement-only preload in ignored
`desktop/dist/index.html` installs a React DevTools commit counter before React
loads and seeds sandbox workbench preferences. It is not part of product source.
A normal frontend/native build removes it after qualification.

## Machine and cases

Measured 2026-10-27 on Windows 11 Home 10.0.26300, Dell Inspiron 5515,
AMD Ryzen 5 5500U (6 cores/12 logical), Radeon Graphics, approximately 16GB RAM,
1920x1080 at 60Hz, 120 DPI / 125%. Actual connected WebView2 reports
`157.0.4341.16`, matching the installed runtime. This was an interactive shared
desktop, without a controlled power/thermal/background-load experiment.

Each build runs the same 12 cases:

- 12 and 1280 synthetic user-library sheets, selected through the native catalog
  and filtered to their fixture prefix. The long selected sheet has 480 notes
  over approximately two minutes; other rows are lightweight one-note sheets.
- Idle and native **dry-run** playback, using the ordinary UI prepare/start/Stop
  path. Native playback state is verified as Playing. No physical gameplay
  keyboard input or game process is involved.
- Inspector 280→440→280 at client width 1366 CSS px; navigator 240→340→240 at
  1366; Inspector 280→requested440→280 with the 388 cap at 1024.
- Each physical pointer triangle lasts four seconds. Helpers request moves every
  ~4ms; Windows/WebView coalesces delivery. JSON retains actual trusted pointer
  event counts, generally around 220–240 per gesture. No mock bridge, synthetic
  DOM pointer stream or per-move IPC drives measured resizing.

The harness uses only SendInput for pointer movement/button input. It
restores/positions its own launched window and verifies foreground ownership;
an activation click is allowed only when WindowFromPoint belongs to that PID.
It refuses to connect if its DevTools port is already occupied. WebView data,
native configuration and song fixtures are temporary sandbox data. It closes
only its launched process after each fixture set. Setup uses DevTools-driven UI;
the measured pointer stream comes from Windows and is checked for `isTrusted`.
No process memory access, game debugger, hooks or alternate input injection was used.

## Measurements and limits

- A preload counts React **root commits during pointer capture**, including
  drag-start state changes and playback-driven commits. Pointerup's final commit
  is outside that counter. Counts are not component render counts or React
  Profiler render durations. Final idle counts of 1–2 demonstrate that distinct
  transient widths no longer require a Workbench React commit per width.
- An rAF sampler records frame intervals and pane geometry while the pointer is
  down. Final and retained baseline matrices both have p50 ~16.7ms and no >33ms
  gaps in these short windows. This does not prove compositor presentation FPS,
  eliminate longer-session stalls or establish VS Code parity.
- Latest-pointer error compares pane DOM width to the last delivered event's
  requested/clamped width. Matched-event age finds the most recent matching
  event within 1px. Both include quantization, sample ordering and instrumentation
  effects. They are **native DOM tracking estimates**, not hardware input-to-photon
  latency. Tracking lag did not show a clear systematic improvement.
- `Performance.getMetrics` supplies script, layout and style durations over the
  trace window, including helper setup and a 100ms settle. DevTools Paint event
  distributions and summed durations are retained separately. Categories may
  overlap; do not add them into an exclusive CPU budget.
- Geometry sampling can itself force layout. The same sampler and commit-counter
  preload are used before/after. These are single-case distributions on one
  machine, without repeated-trial confidence bounds or CI FPS thresholds.
- Some style/layout/paint costs remain above baseline even after registration;
  tables report them rather than masking them with callback counts. The primary
  measured gain is reduced React/script work, not a demonstrated increase in
  already approximately 60Hz rAF cadence or reduced presentation latency.

The intermediate run exposed expensive inherited-property style invalidation.
The final implementation registers the two shell width properties with
`inherits:false`, then explicitly inherits each only at its immediate pane. A
browser assertion protects that content/rows do not inherit transient widths.
This retains the leader-approved shell-property architecture and existing flex,
container queries, virtualization, keyboard, ARIA and focus behavior.

## Reproduction and validation

From `desktop/`, build with `bun run build`, then run
`node scripts/profile-workbench-resize.mjs --prepare=true`. From the repository
root build the measurement shell:

```powershell
cargo build --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell
node desktop/scripts/profile-workbench-resize.mjs --label=final --output=C:/Temp/sky-resize-evidence
```

Measured commands exited 0 for retained before, intermediate and final matrices.
Baseline continuation used `--resume=true --counts=1280` with
`--exe=<frozen-baseline-exe>`; the harness checks executable identity matches.
Do not put output under Playwright's `test-results`: Playwright clears it. Early
attempts hit the native single-subscriber reload contract, incorrectly used
playlist-import fixtures for All Songs, failed foreground ownership, or had an
interrupted pointer stream. Those attempts were rejected, not counted as
successful native measurements. An initial complete baseline's raw files were
accidentally cleared by browser tests; the retained baseline was recorded again
outside that cleanup directory.

Focused validation: 39 workbench unit/component tests; 9 final browser cases;
an earlier three-worker, three-repeat run passed all 27 cases for the same
visual-path/reload behavior before property registration; desktop
typecheck/lint/production build passed. Exact canonical final-HEAD
`cargo xtask check all` command/exit and subsequent normal bundle smoke/native
rebuild are recorded in the PR handoff. Two existing opt-in capture tests remain
opt-in. No meaningful assertions were removed; close/reopen now expects the
persisted width because uncommitted CSS session widths are discarded on unmount.

No game-input timing, native heap, release-mode, caption/Snap or VS Code comparison
acceptance was run. Wave A remains subject to Agent Leader review; B–E remain
unauthorized.

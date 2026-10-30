# Issue #563 — Package D qualification

This report records the whole-feature qualification for Packages A, B, and C on
`feat/desktop-ui-boundaries-graphite`, against
`main@256be1d5e83ef9b7785ece291d1910da854fccb2`. Package C's accepted source is
`c93ab8cf675d47d911e3e66f48b04dbbf65df795`; Package D changes only current
architecture/UI documentation and this evidence report. The package and final
SHA will be recorded in the PR after push.

## Frozen palette decision

- **Graphite A** is the production/default palette. Its persisted ID remains
  `graphite`; the dark color mode remains active.
- **Soft Graphite B** remains a test/evidence-only preview. No B value is
  promoted into production CSS. No alternate palette is introduced.
- The existing Package C A/B artifacts and computed color-pair results are
  preserved in [`issue-563-package-c`](../issue-563-package-c/README.md) and
  [`render-measurements.json`](../issue-563-package-c/render-measurements.json).
  Those browser-mock screenshots do not qualify packaged WebView2 or physical
  Windows high-contrast rendering.

## Integration audit against baseline

| Area | Contract checked | Existing regression evidence | Result |
| --- | --- | --- | --- |
| Library cache and paging | Bounded page cache, generation/request fencing, 100 Built-in results per page, page 101, full-source traversal, selected song independent of playing song | `desktop/src/state/store.test.ts`; `desktop/tests/e2e/library.spec.ts`; `desktop/tests/e2e/library-layout.spec.ts` | Covered; qualification commands/results below |
| Playback and session races | Latest playback intent, pending-start identity, session epoch/retirement, cancellation and recovery; Rust remains authoritative | `desktop/src/state/store.test.ts`; `desktop/tests/e2e/playback.spec.ts`; `desktop/tests/e2e/playback-recovery.spec.ts` | Covered; no native/event-hub changes |
| Workbench geometry | 420 px Track Browser minimum, 46 px track rows, 92 px Player Dock, 800×560 native minimum; dock/overlay topology and clamped resize | Workbench geometry unit tests; `desktop/tests/e2e/workbench-layout.spec.ts`; `workbench-resize.spec.ts`; `workbench-state-regression.spec.ts` | Browser geometry/resize qualification; live packaged resize performance is UNVERIFIED |
| Settings and persistence | Modal navigation/focus, all six persisted palette IDs, v5 layout preference validation, explicit commits only | Settings unit tests; `desktop/tests/e2e/settings-diagnostics.spec.ts`; Workbench preference tests/E2E | Covered; no settings schema change |
| Diagnostics | Presentation remains sourced from native snapshots; dry-run metrics remain “Not measured”; event and trace output remain truthful | Diagnostics component tests; `desktop/tests/e2e/settings-diagnostics.spec.ts` | Covered; browser bridge is a mock |
| Colors and accessibility | Semantic roles remain separate; Graphite A remains production; contrast thresholds and six palettes; keyboard, focus/ARIA, forced-colors, reduced motion | Package C rendered-role tests and A/B evidence; full appearance/accessibility Playwright suites | Browser/CSS qualification; physical high-contrast behavior is UNVERIFIED |
| Native/security boundary | No changes under Rust, Tauri native sources, generated contracts, event hub, updater/release configuration, or native protocol | Full baseline diff/path audit and repository gate | No scope leak found |

The 640×448 responsive fallback and forced-colors/reduced-motion states are
Chromium browser emulations. They are not supported native window sizes or proof
of Windows High Contrast. All native playback/input, timing, and catalog
authority remains unchanged; no gameplay input was sent.

## Packaged Windows smoke and performance limits

Environment: Windows 11 Home `10.0.26300`, x64; AMD Ryzen 5 5500U (6 cores / 12
logical processors), 15.4 GiB RAM; Balanced power plan. Toolchain versions are
recorded in the Package D PR. The optimized desktop executable was built from
the accepted Package C production source (`c93ab8cf675d47d911e3e66f48b04dbbf65df795`);
its SHA-256 was `4379E7FF07F69350486A694215CEAA088A4115C8F98A19B3AB1BEF12CF613A4F`.
The measurement preload was injected into ignored `desktop/dist/index.html`, but
Tauri's `beforeBuildCommand` rebuilt `desktop/dist` and replaced that preload
before compiling the executable. This is one reason the resize profile was not
valid; no instrumentation was committed or included in product source.

- Packaged GUI smoke: `--selftest-desktop-gui` exited 0 using a temporary app-data
  directory. The real packaged WebView loaded `http://tauri.localhost/`, ran the
  smoke dispatch, and completed the controlled shutdown lifecycle. This is a
  safe packaged bridge/settings/diagnostics/calibration smoke, not interactive
  user workflow or game playback proof.
- Startup timing: **UNVERIFIED**. The documented benchmark harness could not
  validate its required `react.catalog_ready` frontend marker. The packaged
  smoke emitted native `catalog.ready` and `catalog.reconciled`, plus
  `react.shell_ready`, but no `react.catalog_ready`; the harness therefore
  rejected the run instead of producing a sample. No baseline comparison or
  startup PASS is claimed.
- Resize timing: **UNVERIFIED**. The WebView2 profiler connected to CDP but its
  page target became a Chromium `chrome-error://chromewebdata/` page showing
  `127.0.0.1 refused to connect`; `.workbench` never became available. The
  profiler produced no valid drag samples or traces. The browser geometry and
  pointer/keyboard resize regression suites remain the evidence for frontend
  behavior; they are not a packaged performance comparison.
- Packaging: Tauri emitted the optimized exe and NSIS bundle, then returned
  exit 1 because the updater public key is configured but the matching private
  signing key is unavailable. No signing key was requested or added. This does
  not qualify a signed installer or updater artifact.

Package C screenshot paths and render/contrast JSON remain the visual evidence
for this package. They are paired A/B captures from the deterministic browser
mock, not packaged screenshots: see the
[Package C evidence index](../issue-563-package-c/README.md) and
[render measurements](../issue-563-package-c/render-measurements.json).

## Qualification results

Environment: Windows 11 Home `10.0.26300`, PowerShell; Bun `1.4.4`, Node
`24.21.0`, Playwright `1.64.1`, Cargo `1.99.1`, rustc `1.99.1`. Hardware and
the Balanced power plan are listed above.

| Command | Exit | Result |
| --- | ---: | --- |
| `bun run typecheck` (from `desktop/`) | 0 | TypeScript check passed. |
| `bun run lint` (from `desktop/`) | 0 | ESLint passed with `--max-warnings 0`. |
| `bun run format:check` (from `desktop/`) | 0 | Prettier passed. |
| `bun run test` (from `desktop/`) | 0 | 37 files, 444 tests passed. |
| `bun run build:web` (from `desktop/`) | 0 | Vite production build passed; main chunk 530.02 kB, above the existing 500 kB advisory. |
| `bun run test:e2e` (from `desktop/`) | 0 | 129 total: 126 passed, 3 skipped opt-in screenshot captures, 0 failed; 2.3 minutes. All functional workbench, Library, Playback, Settings/Diagnostics, palette, focus, forced-colors, reduced-motion, and accessibility suites ran. |
| `cargo xtask check all` (repository root) | 0 | Rust fmt, Clippy, workspace tests, Bun static checks, all E2E, and desktop feature checks passed. Architecture check reported 48 existing allowlisted warnings. The maintained wrapper does not print individual Rust test totals on success; the standalone Bun counts are listed above. |
| `git diff --check` | 0 | No whitespace errors. |

Measurement command outcomes:

- `bun run benchmark:startup -- --exe ..\rust\target\dist\sky_desktop_shell.exe --runs 10 --output ..\docs\evidence\issue-563-package-d\startup-current-c93ab8c.json` — exit 1, required `react.catalog_ready` marker missing; no report file was produced.
- The same command with `--install-root ..` and output `startup-current-c93ab8c-root-install.json` — exit 1 for the same missing marker.
- `bun scripts/profile-workbench-resize.mjs --prepare=true` — exit 0. Its preload was subsequently overwritten by Tauri's `beforeBuildCommand` frontend rebuild.
- `$env:CARGO_TARGET_DIR='D:\Dev\Sky-Auto-Player\rust\target'; bun run tauri build --ci -- --profile dist` — exit 1 after the optimized executable and NSIS bundle were emitted, because updater signing lacked `TAURI_SIGNING_PRIVATE_KEY`.
- `bun scripts/profile-workbench-resize.mjs --exe ..\rust\target\dist\sky_desktop_shell.exe --output ..\docs\evidence\issue-563-package-d --label current-c93ab8c --counts 12,1280` — exit 1. CDP reached a Chromium error page, not the app workbench; the diagnostic run timed out waiting for `.workbench`. No trace or resize measurements are used as evidence.

The three E2E skips were explicit manual-evidence opt-ins in
`graphite-package-c.spec.ts`, `graphite-final-ui-polish.spec.ts`, and
`visual-evidence.spec.ts`; no functional assertion was skipped. The A/B
screenshots remain the Package C browser-mock artifacts. The current package
did not produce new screenshots because the packaged resize profiler could not
reach a valid workbench target.

## Final review checklist

- [ ] Exact pushed Package D SHA recorded in this report and PR #564.
- [x] Full focused and repository gate results recorded above.
- [x] Baseline-to-HEAD changed paths reviewed; no native/generated/schema/release
  changes, cache/page limit drift, selection/playing regression, or palette B
  production values.
- [x] Package D remains on the existing branch and Draft PR #564.
- [ ] Stop for Agent Lead final review; no Package E, merge, or release started.

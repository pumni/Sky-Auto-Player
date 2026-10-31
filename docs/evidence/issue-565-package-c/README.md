# Issue #565 — Package C Windows and system-theme qualification

This evidence separates a real Windows Tauri/WebView2 run from browser media emulation. The
machine-readable report records the immutable source/build SHA, executable SHA-256, runtime, OS
preference snapshot, observed computed styles, contrast results, restart checks, and restoration
state: [`windows-webview2-qualification.json`](windows-webview2-qualification.json).

## Windows Tauri/WebView2 run

The harness built and launched the release-profile Tauri executable with production frontend
assets on Windows 11 Home, build 26300, WebView2 Runtime 157.0.4341.24, Node 24.21.0, and x64.
The primary desktop bounds were 1536×864 at 32 bpp (WMI video mode 1920×1080 at 60 Hz). The
desktop used 125% scaling (`devicePixelRatio=1.25`), and the WebView reported 100% visual viewport
scale with root zoom `1`. This was the real Tauri WebView2 page,
attached with the WebView2 remote debugging port; it did not use the Playwright browser mock or a
Vite dev server. The executable was built with `--no-bundle`, so this run did not install or test
the NSIS installer. It qualifies the release-profile app executable and its packaged frontend
assets, not the installer flow. The harness source/build commit is
`ae1b8469b5081ecad5ac8e803ebf71f0cfa5fe3c`; executable SHA-256 is
`bdaf0ac1f4489b688f9d12626549794bb7ce97a770589e28678579bc89a8d9e6`. The JSON report also records
the Rust toolchain and native build identity.

To change Windows Apps mode while the app was running, the test harness temporarily set the
current user's `AppsUseLightTheme` value and broadcast `WM_SETTINGCHANGE` with
`ImmersiveColorSet`. It restored both `AppsUseLightTheme` and `SystemUsesLightTheme` to their
pre-test values (`0`/`0`) in `finally`; the product does not read or write these registry values.
Each profile used its own temporary native app-data root and WebView2 user-data directory. The
app stayed idle throughout; no gameplay input or playback was sent.

| Check                                                                             | Result                                                                                       |
| --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Empty fresh profile under Windows Apps Light                                      | System mode, Graphite palette, rendered Light; native config persisted `system` / `graphite` |
| Live Windows Apps Light ↔ Dark in System mode                                     | Both transitions updated `html[data-color-mode]` and computed `color-scheme`                 |
| Manual Dark while Windows Apps mode changed                                       | Stayed Dark; native preference persisted                                                     |
| Manual Light while Windows Apps mode changed                                      | Stayed Light; native preference persisted                                                    |
| Return to System and restart                                                      | Followed current Windows Apps mode after both transitions and restart                        |
| Legacy schema 9 config (`palette: slate`, no appearance mode), Windows Apps Light | Migrated to schema 10 `appearance_mode: dark`; rendered Dark and retained Slate              |
| Legacy restart                                                                    | Dark and Slate remained selected                                                             |
| Appearance in WebView localStorage                                                | No appearance/theme keys found                                                               |
| User Windows theme values                                                         | Restored to the recorded `0` / `0` snapshot                                                  |

The captures below are screenshots from the actual Tauri WebView2 page. They are not full desktop
compositor captures.

| Scene                                                        | Screenshot                                    |
| ------------------------------------------------------------ | --------------------------------------------- |
| Fresh profile · System Light · Appearance Settings           | [PNG](fresh-system-light-webview2.png)        |
| Fresh profile · Light · Global Search keyboard focus         | [PNG](fresh-light-search-focus-webview2.png)  |
| Fresh profile · Graphite Dark · Global Search keyboard focus | [PNG](fresh-system-dark-focus-webview2.png)   |
| Legacy Dark / Slate while Windows Apps mode is Light         | [PNG](legacy-dark-windows-light-webview2.png) |

## Rendered contrast and geometry

The harness sampled visible text from computed foregrounds, ancestor backgrounds, and opacity in
the live WebView2 page. Light Settings had 104 samples with minimum 4.97:1; Graphite Dark had 91
samples with minimum 4.94:1. Neither scene had an inactive-control exemption or a text failure.
The actual keyboard focus ring remained visible in both modes: Light `rgb(11, 88, 165)` on white
at 7.11:1; Graphite Dark `rgb(150, 136, 198)` on `rgb(32, 33, 35)` at 5.09:1. At 125% scale,
Chromium reported a 1.6 CSS-pixel outline, equivalent to 2 device pixels. The report contains the
sample counts, lowest text samples, backgrounds, and focus state.

Observed CSS geometry was a 40 px title bar, 46 px track row, 92 px Player Dock, and a 534 px
Track Browser. The document width matched the 786 px WebView client width. The same host run
reported 125% scale and a 100% visual viewport; this is a DPI observation, not a native resize
qualification at every supported size. Browser E2E separately checks the 800×560 layout and
640×448 responsive fallback. The 640×448 case remains browser-only.

## Startup, native background, and limits

`App.test.tsx` delays both native bootstrap and the Tauri theme getter past the 2.5-second
fail-open. It verifies that the bounded fail-open still reveals the pre-bootstrap Windows-media
Light screen, then a loaded legacy Dark setting takes effect and remains Dark when the pending
getter resolves Light. This is frontend/unit evidence; the harness did not inject a hung or delayed
native bootstrap into the packaged executable.

The Tauri window configuration remains RGB `(18, 19, 20)`. A normal Light session in the
release-profile executable reported
white computed HTML/body backgrounds after WebView content loaded. The pre-WebView native surface
was not captured with an OS screen sampler, so any first-frame native-background flash is
**UNVERIFIED**. The fixed color and the 2.5-second fail-open were not changed.

The Windows High Contrast setting itself was not enabled. Forced-colors and reduced-motion
results are Chromium media-emulation checks, not Windows accessibility-setting qualification.
The Windows run did not play a track to create a live `playing` row; selected-versus-playing is
covered by browser E2E. The release-profile executable was not installed from an NSIS bundle.
These cases are not reported as passes.

## Automated regression runs

Commands were run from `desktop` unless noted. The Windows qualification harness and
`cargo xtask check all` were run from the repository root:

| Environment                                                  | Command                                                                                                                                                                                 | Result                                                                                                          |
| ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Windows 11, Node/Bun                                         | `bun run tauri build --ci --no-bundle -- --profile dist`                                                                                                                                | Exit 0; release-profile executable built from the recorded source SHA in 4m23s                                  |
| Windows 11, Node 24.21.0                                     | `node docs/evidence/issue-565-package-c/qualify-windows.mjs --allow-os-app-theme-toggle --output=<temporary-directory>`                                                                 | Exit 0; 8 WebView2 checks passed, 4 screenshots captured, app/system theme values restored                      |
| Windows 11, Node 24.21.0                                     | `bun run test -- src/platform/systemAppearance.test.ts src/platform/startupWindowReveal.test.ts src/app/App.test.tsx`                                                                   | Exit 0; 40 passed                                                                                               |
| Windows 11, Node 24.21.0                                     | `bun run test:e2e -- tests/e2e/light-appearance.spec.ts tests/e2e/palette-appearance.spec.ts`                                                                                           | Exit 0; 12 passed, 1 screenshot-capture-only test skipped                                                       |
| Windows 11, Node 24.21.0                                     | `bun run test:e2e -- tests/e2e/system-appearance.spec.ts tests/e2e/graphite-appearance-regression.spec.ts`                                                                              | Exit 0; 2 passed                                                                                                |
| Linux Chromium, `mcr.microsoft.com/playwright:v1.64.1-noble` | `node scripts/run-e2e.mjs tests/e2e/light-appearance.spec.ts tests/e2e/palette-appearance.spec.ts tests/e2e/system-appearance.spec.ts tests/e2e/graphite-appearance-regression.spec.ts` | Exit 0; 14 passed, 1 screenshot-capture-only test skipped                                                       |
| Windows 11, Node/Bun                                         | `bun run check`                                                                                                                                                                         | Exit 0; typecheck, lint, format, 454 unit tests, and production web build passed                                |
| Windows 11, Rust/Bun                                         | `cargo xtask check all`                                                                                                                                                                 | Exit 0; full repository gate passed, including Rust tests, desktop tests/E2E, Clippy, security and trust checks |

The Windows browser and Linux Chromium runs cover the same Light, palette, System, Graphite,
focus, forced-colors, reduced-motion, geometry, selected-versus-playing, and accessibility
assertions. No assertion was removed or skipped for Linux. The single skip in each appearance E2E
run is the opt-in screenshot-capture-only case. Windows emitted the `NO_COLOR`/`FORCE_COLOR`
environment warning. The production Vite build emitted its existing large-chunk and Rolldown
timing warnings; Vitest emitted its jsdom setup advisory. The full Rust gate reported 49
allowlisted architecture warnings and passed with Clippy warnings denied. The Linux container
printed an npm update notice; tests passed.

The focused native appearance tests cover unsubscription while `onThemeChanged` registration is
pending, including cleanup of the late listener without a late theme getter. The App tests also
cover StrictMode subscription/reveal behavior and a pending native theme getter resolving after the
2.5-second fail-open and legacy settings load.

The Windows WebView2 screenshots above were captured by the harness. Package B's optional
Diagnostics compact browser screenshot was not changed in Package C.

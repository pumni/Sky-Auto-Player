# WP2 UX, accessibility, visual, and performance evidence

## Source identity

- Reviewed foundation: `96de3276e3b3682ddf8158bc72d7bbdc0ee82798`.
- Product implementation: `c0164f3704ae513d6987251ad554daf20a183e70`.
- Forced-colors follow-up: `c420f5f280f339bcaa5d8f83eafe233b5ff129e7` keeps idle track-row borders on the forced-color surface and adds a regression assertion; selected and playing indicators remain intact.
- Browser and packaged-startup profiles are source-bound to `c016`. The only later runtime change is inside the forced-colors media query; the expanded screenshots and canonical captures were regenerated from desktop tree `6419857e8a4b5966fef7f4051948c042f183e93a` at `c420`.
- Docs/evidence commits follow the product-source commit. The manifest records the captured source and desktop-tree identity.
- Screenshots and browser profiles use the deterministic desktop mock. They are not native playback, physical input, screen-reader, or Windows rendering qualification.

## Implementation and the four desktop CI failures

WP2 refined typography, visual hierarchy, button roles, library and playback feedback,
readiness state, Inspector, Settings appearance preview, diagnostics, recovery, dialogs, motion,
reduced-motion handling, and responsive layouts across the actual application. It retained native
playback authority, conversion consent, timing-risk decisions, focus safeguards, preferences,
security boundaries, and existing functionality. The package audit removed the unused `link` and
`lg` Button variants and checked component exports, helper/state duplication, styling, and
architecture remnants. No new dependency or lint suppression was added.

The four findings from Actions run `45506946263`, desktop job `138844730913`, are resolved as
follows:

1. **Pager spacing:** reduced the reserved indicator width using tabular `em` sizing and adjusted
   inner padding. The geometry regression remains active and now meets the 10px maximum.
2. **Axe contrast while closing Add songs:** the closed-state scan now waits for the menu exit
   lifecycle event and confirms removal before scanning. The open-menu scan remains enabled; no
   arbitrary sleep or exception was added.
3. **Axe `aria-hidden-focus`:** D11 permits only the exact six Base UI 1.9.0 focus sentinels in the
   stable Add songs flow to be adjudicated, with complete raw Axe retention, fail-closed
   classification, keyboard behavior, accessibility-tree checks, and negative controls. Details
   and evidence are in the [D11 ledger](../../wp1-r1-coverage-migration.md#wp2-d11-conditional-adjudication).
4. **Inspector heading locator:** scoped the header query to the named top-level `Utility views`
   tablist; nested Performance, Timing, and Events views remain covered.

## D11 Axe evidence

The complete unfiltered Axe 4.14 result is [`axe-full-unfiltered-add-songs.json`](axe-full-unfiltered-add-songs.json),
SHA-256 `7C1A8DE365AF5A6B69F9D9C0CB6F2D3C528AF459F51EECD483476D6B6561C1CB`. It has 2 violations:

- `aria-hidden-focus`, serious, six Base UI sentinel nodes, conditionally adjudicated only for the
  approved stable open Add songs flow.
- `region`, moderate, one finding, retained and not adjudicated.

The report also records 4 incomplete rules, 43 passes, and 46 inapplicable rules. This is not a
zero-violation Axe result. The separate
[`axe-focus-guard-adjudication.json`](axe-focus-guard-adjudication.json), SHA-256
`FE64A7D22E8FF875C99016CA57878AD61DBC8170D0559E3B9C83E97CEACB6AC9`, contains the sentinel
classification and measured focus traversals. The test verifies Tab and Shift+Tab leave the popup
within one second, menu/keyboard lifecycle, and negative controls. It adds no rule/selector
exclusion, production DOM mutation, blanket suppression, or weakened Axe assertion.

## D12 performance qualification

### Browser profile

Profiles compare reviewed `96de` to product implementation `c016` on Windows 11, Node 24.21,
Chromium 156.0.8078.4, 1366×768 CSS viewport, DPR 1, deterministic mock, and production Vite
preview. Each profile has one discarded warm-up and ten measured fresh browser contexts with the
browser process reused; interaction timing uses five measured repetitions through a stable state
and two animation frames. Full raw samples, trace timings, resource bytes, memory, frame intervals,
long tasks, and accessibility-tree observations are retained.

| Metric | 96de baseline | WP2 product source | Change |
| --- | ---: | ---: | ---: |
| Shell ready p50 / p95 | 469.3 / 541.2 ms | 392.2 / 515.8 ms | lower |
| Catalog ready p50 / p95 | 477.3 / 554.4 ms | 399.6 / 526.2 ms | lower |
| Initial loaded JS, encoded | 250,059 bytes | 252,140 bytes | +0.83% |
| Initial loaded JS, decoded | 801,050 bytes | 807,590 bytes | +0.82% |
| First-load ScriptDuration | 254.76 ms | 269.12 ms | +5.6% |
| First-load TaskDuration | 476.84 ms | 496.61 ms | +4.1% |
| First-load LayoutDuration | 135.23 ms | 141.14 ms | +4.4% |
| Heap | 10.45 MiB | 10.54 MiB | +0.09 MiB |
| Long tasks ≥50 ms | 8 | 8 | unchanged |
| Longest task | 162 ms | 143 ms | lower |

Interaction p50 / p95 in milliseconds (baseline → WP2):

| Interaction | 96de | WP2 |
| --- | ---: | ---: |
| Settings open | 76.7 / 389.5 | 87.0 / 373.6 |
| Settings close | 174.4 / 259.6 | 159.7 / 278.1 |
| Add songs open | 59.5 / 93.7 | 76.3 / 92.7 |
| Add songs close | 127.4 / 128.5 | 127.6 / 145.4 |
| Inspector open | 60.2 / 92.6 | 61.3 / 88.7 |
| Inspector close | 44.8 / 75.8 | 57.8 / 59.3 |
| Virtualized scroll | 84.5 / 118.1 | 85.5 / 103.0 |
| Viewport resize | 32.9 / 34.6 | 31.2 / 38.6 |

The samples overlap; p95 is the maximum of five interaction repetitions, and frame interval p50/p95
stayed near 16.7/33.3 ms. There is no repeatable greater-than-10% startup, list, or resize
regression. This lightweight browser profile is comparative evidence, not a native 60 Hz claim.
Raw baseline: [`wp2-browser-profile-baseline.json`](wp2-browser-profile-baseline.json), SHA-256
`FEEC2CE4AB0FFD45684189FE999DD2ADB012BF50A8CCFFC2A81ECFBE7EC76195`. Raw candidate:
[`wp2-browser-profile-candidate.json`](wp2-browser-profile-candidate.json), SHA-256
`CED07DEAEA5CE2D525B9897FA03B8DA07177CFB45B185C0AF86768E2840328FE`.

### Bundle and native startup

The D12 bundle ceilings were 670 kB raw / 205 kB gzip for the main JS asset and 900,000 bytes for
all emitted JS. Final product output was 658.05 kB raw / 200.22 kB gzip for main JS, and 881,120
emitted JS bytes versus 874,071 at baseline (+0.81%). CSS was 75.95 kB versus 76.34 kB. The main
asset retains Vite's 500 kB advisory; the bundle was not artificially split to chase parity.

The reviewed 96de packaged startup sample is invalid for a baseline comparison: it lacks the
`react.catalog_ready` telemetry marker. The startup harness failure was investigated and candidate
instrumentation now records catalog readiness after native reconciliation and flushes telemetry
before smoke shutdown. Arbitrary bundle-parity rebuilding was not pursued.

The candidate profile uses ten measured runs plus one warm-up per mode and fixture. “Coldish” means
a fresh app-data root and manifest without clearing Windows filesystem/OS caches. “Warm” reuses one
app-data root but restarts the process for each sample. Fixtures contain 0, 12, and 1,280 imported
entries. The table gives process-to-shell-ready and catalog-reconciled p50 / p95 in milliseconds:

| Fixture | Mode | Shell ready | Catalog reconciled |
| --- | --- | ---: | ---: |
| Minimal (0) | Coldish | 859.393 / 1,104.021 | 1,128.804 / 1,373.425 |
| Minimal (0) | Warm | 953.983 / 1,097.076 | 1,316.764 / 1,487.947 |
| Representative (12) | Coldish | 826.754 / 1,113.228 | 1,053.261 / 2,256.783 |
| Representative (12) | Warm | 769.019 / 821.613 | 1,079.506 / 1,218.267 |
| Large (1,280) | Coldish | 907.957 / 1,060.140 | 1,299.779 / 1,578.220 |
| Large (1,280) | Warm | 904.537 / 962.032 | 1,022.029 / 1,092.632 |

The representative coldish catalog sample range is 1,282.646 ms, so the p95 is noisy. The profile
is preserved in [`d12-wp2-candidate-startup.json`](d12-wp2-candidate-startup.json), SHA-256
`FBD28321BC626E864B16CE2C1B55D89D4329E5D2C80C791BFB00FA9CF90791E1`. The Tauri build produced a
candidate package, then exited at the unsigned signing step because no private signing key was
available. No signing credentials were requested or used, and the candidate was not installed.
Following the owner's direction, the existing install remains at the reviewed 96de package.

## Visual review

[`expanded-visual-evidence.zip`](expanded-visual-evidence.zip) contains 25 full-size PNGs and
`candidate-measurements.json`, SHA-256 `56665FD72943A05302F4349B0362275CFC34A7AFA757597DD79A19892D0351E6`.
It covers 640, 800, 1,024, 1,200, 1,366, and 1,920 px viewports; all six dark palettes and Light;
appearance preview; Runtime; admission; update; empty and recovery-error states; forced colors;
reduced motion; and Add songs open/keyboard interaction. The four recommended deterministic current
scenes are in [`../../current/`](../../current/), with their exact final source and desktop-tree
identity in [`../../current/manifest.json`](../../current/manifest.json).

The captures were visually inspected at product-source commit `c420`. The compact overlay uses the responsive
fallback at 640×448, and the wide scene shows the docked Inspector; typography, readiness status,
content hierarchy, selection/playback state, and controls remain readable. The forced-colors view
confirms idle row edges blend into the track surface while system-color focus remains visible. They
are mock-browser captures, not native Windows screenshots.

## Verification and remaining limits

`node scripts/toolchain-pin.mjs exec -- cargo xtask check all` passed on the completed WP2 handoff
tree with product source at `c420`. This includes the repository verification path, Rust
checks/tests, desktop checks, `bun run check`, and the full Playwright E2E suite. After the
forced-colors follow-up, its focused E2E passed; the targeted expanded screenshot capture,
canonical `bun run ui:refresh`, and production-bundle E2E also passed at `c420`. The architecture
contract reports 68 existing advisory warnings; no suppression was introduced. The PR handoff
comment records the exact checked HEAD and artifact hashes.

Manual Narrator/NVDA checks were not run. No physical gameplay input or ReceiveOnly qualification
was performed. The candidate package was not installed or signed. CI was not awaited before this
checkpoint handoff, as directed.

# Wave D Playwright runtime comparison

The runs below compare the complete browser suite on one fixed Wave D source tree. All six runs were sequential and used the same local runner and Windows machine. No CI worker setting changed, and no fake clock was used as a performance measurement.

## Input and runner

- Wave D source commit: `5ceed7e03a8260f56e5d75f22f1dc5a76847deec`
- Wave D source tree: `b91c3bee0cfc09d5211be2856059f27b24b6ae26`
- Command for every run: `bun run test:e2e -- --workers=N --reporter=dot` from `desktop`
- The repository runner starts Vite, waits for the local server, and invokes Playwright 1.64.1. Runtime includes that startup and the complete Playwright command.
- Runner versions: Bun 1.4.4, Node.js 24.21.0, Vite 8.3.4, Playwright 1.64.1.
- Machine: Windows 11 Home 10.0.26300; AMD Ryzen 5 5500U, 6 cores / 12 logical processors; 16.1 GB installed RAM (15.4 GiB visible to Windows).
- Each final run listed 106 tests: 104 passed and 2 opt-in visual capture tests skipped. There were 0 failed tests and 0 retries across all six runs.
- `Win32_Processor.LoadPercentage` is an instantaneous coarse load reading. Available RAM and system Chrome/Edge process counts are included as context; other Edge activity varied slightly during the sequence.

## Raw runs

| Workers | Run | Wall seconds | Exit | CPU load before → after | Free RAM before → after (GiB) | Chrome/Edge processes before → after | Result                |
| ------: | --: | -----------: | ---: | ----------------------: | ----------------------------: | -----------------------------------: | --------------------- |
|       2 |   1 |       211.95 |    0 |               24% → 30% |                     6.6 → 6.8 |                              24 → 24 | 104 passed, 2 skipped |
|       2 |   2 |       191.99 |    0 |                28% → 1% |                     6.7 → 6.7 |                              24 → 22 | 104 passed, 2 skipped |
|       2 |   3 |       181.99 |    0 |                 5% → 4% |                     6.8 → 7.0 |                              22 → 22 | 104 passed, 2 skipped |
|       4 |   1 |       135.49 |    0 |                 2% → 4% |                     7.0 → 7.0 |                              22 → 22 | 104 passed, 2 skipped |
|       4 |   2 |       133.37 |    0 |                 5% → 1% |                     7.0 → 7.0 |                              22 → 22 | 104 passed, 2 skipped |
|       4 |   3 |       135.33 |    0 |                 1% → 5% |                     7.0 → 7.3 |                              22 → 22 | 104 passed, 2 skipped |

Median wall time was 191.99 seconds at 2 workers and 135.33 seconds at 4 workers. Four workers were 29.5% faster than two workers on this machine. The spread was 29.96 seconds for 2 workers and 2.12 seconds for 4 workers; no run failed or retried.

## Before/after reference

The pre-Wave-D full browser run used the same command with 2 workers on commit `e4ced422b098aa2ee78d30fb46a6b34ae3d1ccbd`, tree `d79cd7a2d6d0372356c3e0cb4839ab555008f1a3`. That commit added only the pre-change ownership map; its E2E source matched accepted Wave C. It completed 103 passed and 2 skipped (105 total) in 277.13 seconds, including Vite and runner startup. Its recorded free RAM before the run was 5.8 GiB.

The Wave D 2-worker median is 85.14 seconds (30.7%) below that pre-change run, while adding one shared-geometry test. The two historical and final test runs contain the same opt-in capture skips. The final source tree is fixed for all six comparison runs.

## Focused checks

These checks ran on the same E2E source snapshot identified above:

| Command                                                                                                                                                                                                                                                                                                                                                                                                                                             | Exit | Coverage/result                                                            |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---: | -------------------------------------------------------------------------- |
| `bun run typecheck`                                                                                                                                                                                                                                                                                                                                                                                                                                 |    0 | Desktop TypeScript typecheck.                                              |
| `bun x eslint tests/e2e/testHelpers.ts tests/e2e/library.spec.ts tests/e2e/library-layout.spec.ts tests/e2e/workbench-layout.spec.ts tests/e2e/playback.spec.ts tests/e2e/playback-recovery.spec.ts tests/e2e/settings-diagnostics.spec.ts tests/e2e/palette-appearance.spec.ts tests/e2e/workbench.spec.ts tests/e2e/workbench-resize.spec.ts tests/e2e/workbench-state-regression.spec.ts tests/e2e/graphite-row-layout.spec.ts --max-warnings 0` |    0 | All touched E2E TypeScript files; no issues.                               |
| `bun x prettier --check tests/e2e/testHelpers.ts tests/e2e/library.spec.ts tests/e2e/library-layout.spec.ts tests/e2e/workbench-layout.spec.ts tests/e2e/playback.spec.ts tests/e2e/playback-recovery.spec.ts tests/e2e/settings-diagnostics.spec.ts tests/e2e/palette-appearance.spec.ts tests/e2e/workbench.spec.ts tests/e2e/workbench-resize.spec.ts tests/e2e/workbench-state-regression.spec.ts tests/e2e/graphite-row-layout.spec.ts`        |    0 | All touched E2E TypeScript files formatted.                                |
| `bun run test:e2e -- --list`                                                                                                                                                                                                                                                                                                                                                                                                                        |    0 | 106 Playwright cases in 18 files.                                          |
| `bun run test:e2e -- --workers=1 --reporter=dot tests/e2e/playback-recovery.spec.ts`                                                                                                                                                                                                                                                                                                                                                                |    0 | 3 clock-controlled Retry/Stop/no-session recovery tests passed in 5.3 s.   |
| `bun run test:e2e -- --workers=1 --reporter=dot tests/e2e/library-layout.spec.ts`                                                                                                                                                                                                                                                                                                                                                                   |    0 | 5 geometry/layout tests passed in 12.1 s, including six palettes.          |
| `bun run test:e2e -- --workers=1 --reporter=dot tests/e2e/palette-appearance.spec.ts`                                                                                                                                                                                                                                                                                                                                                               |    0 | 5 palette, contrast, modal focus and forced-colors tests passed in 22.6 s. |

## Final repository gate

`cargo xtask check all` ran once on commit `5ceed7e03a8260f56e5d75f22f1dc5a76847deec`, tree `b91c3bee0cfc09d5211be2856059f27b24b6ae26`: exit 0 in 413.25 seconds. Static/security checks, Rust formatting/clippy/workspace tests, desktop checks, and the xtask browser E2E phase passed.

After adding this evidence, `bun x prettier --check` on both Wave D evidence files and `cargo xtask check static` passed. No E2E source changed after the full `check all` run.

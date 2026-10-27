# Wave D assertion ownership — after changes

This map follows the pre-change inventory in [`assertion-ownership-before.md`](assertion-ownership-before.md). The baseline contained 63 declarations in `library.spec.ts`. A title-level comparison against the accepted Wave C source confirms that all 63 now appear exactly once across the split owners. One new cross-palette geometry declaration was added.

## Owners

| Owner | Declarations | Assertions owned after the split |
| --- | ---: | --- |
| `desktop/tests/e2e/library.spec.ts` | 18 | Search and selection; playlist creation, import, and membership; selected-versus-Now-Playing identity; row actions; virtualized paging, keyboard traversal, and mounted `aria-activedescendant`; long-title containment. |
| `desktop/tests/e2e/library-layout.spec.ts` | 5 | Independent fixed Library and Track Browser geometry across all six palette IDs; Graphite color and header behavior; compact rail keyboard access; full row/tile geometry; v5 automatic compact preference. |
| `desktop/tests/e2e/workbench-layout.spec.ts` | 8 | Minimum viewport bounds; dock/overlay behavior; viewport geometry matrix; centered titlebar and Player Bar; separator persistence; accessibility and focus. |
| `desktop/tests/e2e/playback.spec.ts` | 21 | Transport affordances and geometry; natural finish and Auto Play handoffs; frozen playback context; selected-versus-playing; dry-run and physical labeling; Stop, Shuffle, and duplicate-start behavior; immediate startup failures. |
| `desktop/tests/e2e/playback-recovery.spec.ts` | 3 | Status Retry, explicit Stop during recovery, and authoritative no-session recovery. `page.clock` is installed before the app is navigated to, then the mock timeout is advanced by 15 seconds. |
| `desktop/tests/e2e/settings-diagnostics.spec.ts` | 9 | Settings, profile, modal accessibility, palette round-trip, diagnostics pane/drawer and resize, update states, and calibration. |

The other owners listed in the before map remain in place: the integrated workbench smoke, six-palette appearance/contrast and modal focus, Graphite appearance and row affordances, native caption browser checks, opt-in visual evidence, production bundle lazy-chunk smoke, and Rust feature/security checks.

## Retained coverage and ownership changes

- The six-palette geometry test uses fixed expected values for the 40/32/46 px Track Browser header/column/song rows, 240/48 px expanded/collapsed navigator, 42 px Library rows, 24 px icons, and 40 px compact tiles. It does not call the production layout solver to derive its oracle. Existing Graphite color assertions remain palette-specific.
- All original Library test titles and their unique assertions remain in one owner. The three delayed recovery tests moved to the clock-controlled file; the immediate target-startup failure remains in the live-clock playback file.
- The integrated workbench journey remains one test. Repeated Details journeys at five viewport sizes and duplicate Runtime journeys were reduced to one desktop journey. Responsive geometry and dock/overlay state remain asserted in the independent workbench and resize matrices. The integrated journey still includes one real-clock recovery case.
- In the six-palette token/contrast test, stable token maps and resolved-color reads are batched per browser evaluation and still compare against the same explicit token values and contrast thresholds. Interaction-state assertions continue polling where state transitions can be asynchronous.
- The repeated identical `Mode: Physical input` assertion was removed. No unique assertion was removed.
- Screenshots without a matching assertion are disabled in normal runs. Their opt-in flags are `WORKBENCH_INSPECTOR_CAPTURE=1`, `WORKBENCH_RESIZE_CAPTURE=1`, and `GRAPHITE_ROW_LAYOUT_CAPTURE=1`. Existing `GRAPHITE_WORKSPACE_CAPTURE`, `GRAPHITE_FINAL_UI_CAPTURE`, and palette capture gates remain intact.
- Split-file setup no longer clears and reloads each isolated Playwright context. Clock-controlled recovery tests install the clock before navigation. The integrated smoke continues clearing local storage between separate scenarios in its shared page.
- `TRANSPORT_OPERATION_TIMEOUT_MS` remains 15,000 ms. No application, Rust, native input, IPC, persistence schema, or CI worker configuration was changed.

## Assertion inventory

| Behavior contract | Owner after Wave D |
| --- | --- |
| Appearance color roles, contrast, forced colors, reduced motion | `palette-appearance.spec.ts`, `graphite-appearance-regression.spec.ts`, `graphite-final-ui-polish.spec.ts` |
| Shared Library and workbench geometry, six-palette parity, resize and v5 persistence | `library-layout.spec.ts`, `workbench-layout.spec.ts`, `workbench-resize.spec.ts`, `workbench.spec.ts` |
| Library search/CRUD/import, virtual keyboard and ARIA descendant | `library.spec.ts` |
| Playback ownership, selected-vs-playing, explicit Stop and recovery | `playback.spec.ts`, `playback-recovery.spec.ts`, `workbench-state-regression.spec.ts` |
| Browser titlebar safe area and 40 logical px native caption evidence | `titlebar.spec.ts`, `docs/evidence/issue-554/` |
| Production lazy-chunk smoke | `desktop/tests/e2e-bundle/bundle.spec.ts` and its runner |
| Rust feature matrix and input/security boundary | `cargo xtask check all`, Rust integration tests, and static checks |

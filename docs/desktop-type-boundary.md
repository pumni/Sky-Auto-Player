# Desktop Frontend and Type Boundary

The desktop UI uses Tailwind CSS v4, Base UI, and source-owned shadcn/ui components configured
with the official Base UI registry and the `base-vega` starting style. `apps/desktop/components.json`
records the registry style and aliases. The generated/adapted primitives live in
`apps/desktop/src/components/ui/`; they are ordinary reviewed TypeScript source, not a runtime
registry or a second component package.

## Qualified frontend versions

- `tailwindcss` 4.3.5 and `@tailwindcss/vite` 4.3.5
- `@base-ui/react` 1.9.0
- `@axe-core/playwright` 4.14.0 for the pinned Add Songs browser review
- `class-variance-authority` 0.7.1 and `cn` 0.5.2
- `react` and `react-dom` 19.3.1 with `@types/react` 19.3.0
- `typescript` 7.0.2
- `vite` 8.3.4

The exact package versions are pinned in `apps/desktop/package.json` and `apps/desktop/bun.lock`.
The shadcn CLI is an authoring tool only and is not a desktop runtime dependency. WP1 used the
exact pinned CLI invocation `bunx --bun shadcn@4.8.3 add input --yes` to add the official Base UI
Vega input primitive; the generated source was reviewed and adapted to the semantic token system.

## Styling and component ownership

`apps/desktop/src/styles/globals.css` is the one frontend stylesheet entry, imported by
`src/main.tsx`. It owns reset/base integration, Sky Graphite 2 semantic tokens, six dark palette
overrides, Light overrides, forced-colors rules, reduced-motion rules, and genuine measured
workbench geometry. `@theme inline` maps semantic CSS properties to Tailwind utilities; the Tailwind
dark variant follows `html[data-color-mode]`. Feature layout and state styling use utilities and
shared component variants instead of another feature stylesheet or a large `@apply` recreation.

The shared `Button` component owns primary, outline, secondary, ghost, and destructive action
states. Tailwind maps `--primary-hover`, `--primary-pressed`, `--destructive-hover`,
`--destructive-pressed`, and `--scrim` alongside the base color tokens. Production actions should
use these variants instead of recreating button rest/hover/pressed colors in feature markup; native
window-caption controls keep their platform-specific behavior.

The Base UI wrappers provide shared behavior and `data-slot` ownership for buttons, dialogs,
alert dialogs, dropdown menus, inputs, selects, switches, tabs, tooltips, and popovers. Feature
components compose these wrappers and retain domain-specific state, labels, and actions. Browser
selectors should use roles, accessible names, state attributes, or stable `data-slot` values rather
than implementation classes.

Base UI owns primitive keyboard interaction, popup lifecycle, and dialog focus behavior. Feature
components own application actions through normal React callbacks (`onClick`, `onValueChange`, and
related typed props). Avoid fixed JavaScript delays to imitate popup exit lifecycle. Preserve
accessible names, keyboard operation, focus restoration, portal theme inheritance, reduced-motion
behavior, and forced-colors feedback.

Dropdown Menu roots use non-modal behavior for ordinary action menus. Modal dialogs retain their
Base UI focus ownership; the compact Inspector is a non-modal workbench pane and applies `inert`
plus `aria-hidden` immediately on close while its CSS exit completes. All popup transition utilities
include the animated properties and reduced-motion override. The ordinary Add songs menu keeps
Base UI 1.9.0 `modal={false}` behavior. The E2E checkpoint runs the full, unfiltered Axe 4.14.0 scan
and retains its raw report. Its one serious `aria-hidden-focus` finding is conditionally adjudicated
only when the result matches the exact six selectors in the
[`Add Songs adjudicator`](../apps/desktop/src/testing/baseUiFocusGuardAdjudication.ts): two untyped guards and
two `data-type="outside"` guards beside the trigger, plus two `data-type="inside"` guards adjacent
to the open portal menu. Each traversal and the Axe scan records a separate six-guard snapshot,
with expected placement and identity. Instrumented real Tab must close the menu and land on the
visible, enabled “Browse Built-in Songs…” button in the empty playlist state; Shift+Tab must close
it and return to the “Add songs” trigger. The traversal evidence correlates each sentinel focus
event to the measured DOM guard identity and placement; the harness does not synthesize guard
focus. Guard clearance and destination must occur within one second. Version drift,
selector/count/placement changes, mixed findings, hidden or stuck focus targets, and unrelated
serious/critical Axe findings fail closed and have negative unit tests.

The same raw Axe report contains a moderate `region` finding on the Base UI portal menu outside the
page's landmark tree. It remains visible in the unfiltered report and is not suppressed or included
in the serious-finding adjudication. No production DOM or Axe configuration is changed. This
conditional review is pinned to Base UI 1.9.0 and Axe 4.14.0; remove it if either version changes or
the exact finding no longer appears. The historical WP1/WP2 suite migration and accepted raw Axe
files are indexed in [`HISTORY.md`](HISTORY.md) and [`ui/README.md`](ui/README.md). Manual Narrator
and NVDA qualification is NOT RUN by this browser test.

## Type and platform boundaries

Type checking uses the pinned desktop toolchain with strict compiler options, including
`exactOptionalPropertyTypes`, `noUncheckedIndexedAccess`, `noUnusedLocals`, and `skipLibCheck`.
React components remain typed against the published React and Base UI declarations; do not add
module shims, `any` casts, `@ts-ignore`, or relaxed compiler options to bypass type errors.

The UI uses the typed desktop bridge and application state. Imports from `@tauri-apps/**` stay
inside `src/bridge/` and `src/platform/`, enforced by the existing Biome/ESLint boundary checks.
Frontend presentation changes do not change native playback authority, settings schemas, IPC
contracts, Windows `SendInput`, focus qualification, or cleanup behavior.

## Regression coverage and commands

The component tests under `src/components/ui/` cover the source-owned primitives and keyboard or
selection behavior. `src/appearance/graphiteThemeTokens.test.ts` checks the semantic palette
contract. `tests/e2e/theme-accessibility.spec.ts`, `settings-diagnostics.spec.ts`,
`playback-confirmation-layout.spec.ts`, `update-modal-geometry.spec.ts`, and the workbench and
library suites cover rendered tokens, dialogs, selection, focus, responsive geometry, and key
application states. The revision-bound removed Graphite and Light suite migration is indexed in
[HISTORY.md](HISTORY.md).

Run from `apps/desktop/` with the pinned Bun version:

```powershell
bun install --frozen-lockfile
bun run typecheck
bun run lint
bun run test
bun run build:web
bun run test:e2e
```

`bun run check` runs typecheck, Biome and ESLint, format checking, tooling contract tests, Vitest,
and the production web build. The root CI and Rust validation contracts remain in
`docs/ci-validation-topology.md` and `docs/rust-toolchain-policy.md`.

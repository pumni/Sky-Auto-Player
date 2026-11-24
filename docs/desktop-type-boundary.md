# Desktop Frontend and Type Boundary

The desktop UI uses Tailwind CSS v4, Base UI, and source-owned shadcn/ui components configured
with the official Base UI registry and the `base-vega` starting style. `apps/desktop/components.json`
records the registry style and aliases. The generated/adapted primitives live in
`apps/desktop/src/components/ui/`; they are ordinary reviewed TypeScript source, not a runtime
registry or a second component package.

## Qualified frontend versions

- `tailwindcss` 4.3.5 and `@tailwindcss/vite` 4.3.5
- `@base-ui/react` 1.9.0
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
include the animated properties and reduced-motion override. Axe 4.14 currently reports Base UI
1.9.0's `aria-hidden`/`tabindex="0"` Menu focus guards while an ordinary menu is open; this known
R5 finding is recorded in the [WP1-R1 coverage migration ledger](ui/wp1-r1-coverage-migration.md).

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
application states. The deleted Graphite and Light suite migration is mapped in the
[WP1-R1 coverage migration ledger](ui/wp1-r1-coverage-migration.md).

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

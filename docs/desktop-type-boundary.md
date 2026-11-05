# Desktop UI Type Boundary

`desktop/` consumes `react-aria-components` through that package's published declarations. There is
no repository-local module declaration or `tsconfig` path alias for the package. Type checking uses
the pinned desktop toolchain and the existing strict compiler options, including
`exactOptionalPropertyTypes`, `noUncheckedIndexedAccess`, `noUnusedLocals`, and `skipLibCheck`.

## Qualified versions

- `react-aria-components` 1.22.1
- `typescript` 7.0.2
- React and React DOM 19.3.1 with `@types/react` 19.3.0

These are the versions in `desktop/package.json` and `desktop/bun.lock`. The type boundary does not
require a dependency upgrade.

## React Aria surface

Desktop source and tests import these published exports:

- `Button`, `Dialog`, `DialogTrigger`, `Menu`, `MenuItem`, `MenuTrigger`, `Modal`, `ModalOverlay`,
  `Popover`, `Tab`, `TabList`, `TabPanel`, `Tabs`
- `Tooltip` and `TooltipTrigger` from `react-aria-components/Tooltip`

| Area | Importing files |
| --- | --- |
| Buttons | `AppTitleBar`, `LibraryAddMenu`, `LibraryNavigator`, `PlaylistNavItem`, `PlayerTools`, `TrackActionsMenu`, `PlaylistAddSongsMenu`, `TrackBrowser` |
| Menus and popovers | `PlaylistNavItem`, `PlayerTools`, `TrackActionsMenu`, `PlaylistAddSongsMenu` |
| Dialogs and modals | `CalibrationDialog`, `LibraryConfirmDialog`, `PlaylistCreateDialog`, `PlaylistRenameDialog`, `SettingsPanel`, `UpdateDialog`, `PlayerTools` |
| Tabs and panels | `UtilityPane`, `UtilityHeader`, `DiagnosticsView`, `DiagnosticsEventsPanel`, `DiagnosticsPerformancePanel`, `DiagnosticsTimingPlot`, plus `DiagnosticsEventsPanel.test` |
| Tooltips | `LibraryNavigator` |

Files are under `desktop/src/components/` unless the name ends in `.test`. `TabPanel` is used by the
Diagnostics sub-panels, `Tabs` owns the Utility and Diagnostics tab collections, and
`Dialog`/`ModalOverlay`/`Modal` own the modal dialogs. `Tooltip` is used only by the collapsed
library navigator.

## Audit baseline

The removed shim was `desktop/src/types/react-aria-components.d.ts`, selected through
`tsconfig.json#compilerOptions.paths`. It declared a hand-written subset of `Button`,
`DialogTrigger`, `MenuTrigger`, `Menu`, `MenuItem`, `Popover`, `ModalOverlay`, `Modal`, `Dialog`,
`Tabs`, `TabList`, `Tab`, and `TabPanel`.

The only React Aria call-site cast removed was the `Tabs as unknown as ComponentType<...>` alias in
`UtilityPane`. Remaining `as unknown as` casts in desktop source are test-only fixtures for
`MediaQueryList`, store snapshots, and prepared playback; they do not mask React Aria declarations.

With the shim removed at baseline `8af1bbb1`, TypeScript reported 12 errors across 7 files: seven
non-rendered `title` props, one optional `aria-current`, one inert `Popover` id, two `Key` string
operations, and one `Tabs` selection callback. All were corrected without compiler or runtime
suppression.

## Call-site rules

- Use React Aria `onPress` for `Button`, and keep native click handlers on native `<button>`
  elements.
- `Tabs.onSelectionChange` receives `Key` (`string | number`). Narrow the key to the concrete view
  before passing it to store actions.
- `Menu.onAction` also receives `Key`. Narrow string menu IDs before parsing IDs such as
  `playlist:<id>`.
- React Aria buttons do not forward arbitrary HTML attributes such as `title`; the package's
  `filterDOMProps` allowlist excludes it. Use `aria-label` for the accessible name and React Aria
  `Tooltip` when a visible hover hint is required. Do not reintroduce `title` on React Aria
  components.
- Apply optional ARIA attributes through conditional spreads when `exactOptionalPropertyTypes`
  rejects an explicit `undefined` value.
- Do not add a module declaration shim, `any` cast, `@ts-ignore`, or relaxed compiler option to make
  a React Aria call site compile.

## Behavior boundary

The Utility pane keeps its existing docked/overlay presentation, Escape handling, focus restoration,
and lazy `DiagnosticsView` boundary. Tab selection still updates the store and the selected tab keeps
focus through dock and overlay changes. Removing non-rendered `title` props and the inert `Popover`
`id` does not change the rendered DOM because React Aria filters those attributes.

Utility regression coverage lives in
`desktop/src/components/utility/UtilityPane.test.tsx` and the workbench Playwright specifications.
The unit test covers tab selection, keyboard tab movement, lazy Diagnostics mount, and close focus
restoration. `desktop/tests/e2e/workbench.spec.ts` covers dock/overlay transitions, tab keyboard
navigation, focus retention, and Inspector geometry.

## Qualification commands

Run from `desktop/` with the pinned Bun version:

```powershell
bun install --frozen-lockfile
bun run typecheck
bun run lint
bun run test
bun run build:web
```

The `build:web` output keeps `DiagnosticsView` in its own lazy chunk. No manual Vite/Rolldown chunk
configuration is part of this boundary.

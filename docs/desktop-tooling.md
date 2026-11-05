# Desktop JavaScript tooling

The repository-root `.bun-version` is the canonical Bun pin (`1.4.4`). The
`site/package.json` `packageManager` value is a checked mirror used by site
tooling. Desktop and site retain separate manifests and lockfiles:
`desktop/package.json` with `desktop/bun.lock`, and `site/package.json` with
`site/bun.lock`. There is no root Bun workspace manifest or root lockfile.

Run Bun commands from the owning app directory and keep installs frozen:

```powershell
# From the repository root
bun scripts/verify-bun-pin.mjs

# From desktop/
bun install --frozen-lockfile
bun run check
```

## Desktop formatter and lint gates

Biome 2.6.5 is the desktop formatter and general JavaScript, TypeScript, and
React linter. Its formatter matches the prior Prettier settings: two spaces,
100 columns, single quotes in JavaScript/CSS, double quotes in JSX, semicolons,
all trailing commas, and LF endings. Existing generated/build/report ignores
are explicit in `desktop/biome.json`; Rust/Tauri implementation files under
`src-tauri/` stay outside the JS tool surface.

| Existing behavior | Current gate | Coverage decision |
| --- | --- | --- |
| Prettier formatting | `biome format` / `biome format --write` | Preserve the 100-column, quote, semicolon, comma, indentation, and line-ending policy. The desktop `.prettierignore` paths are represented by Biome's scoped file includes. |
| ESLint recommended JavaScript rules | Biome recommended JS/TS rules plus the retained ESLint gate | ESLint remains blocking while Biome becomes the primary general linter. Existing ESLint coverage is retained unchanged. |
| `eslint-plugin-react-hooks` recommended rules | React Hooks rules in Biome plus blocking ESLint | Biome checks top-level Hooks. ESLint remains the authoritative gate for the complete Hooks and React Compiler rule set. |
| Tauri import boundary | Biome and ESLint `no-restricted-imports` | Both reject `@tauri-apps/**` imports under `src/` except `src/bridge/**` and `src/platform/**`. Generated bridge output remains ignored. |

Biome does not run its CSS linter in this package; the approved scope is general
JS/TS/React linting, and the existing authored CSS cascade includes deliberate
specificity and `!important` use. CSS formatting remains enabled. Biome's
`useReactCompiler` rule is not enabled because it is experimental in 2.6.5;
ESLint's pinned `eslint-plugin-react-hooks` recommended config remains a
blocking gate and is covered by negative tests for Hooks and Compiler rules.

The audited exceptions keep the existing enforcement surface stable while
Biome is introduced: `a11y` lint diagnostics are not used as a substitute for
the existing axe/browser accessibility checks; Biome's exhaustive-dependency
port and component-hook-factory rule remain covered by ESLint because the pilot
reported mismatches against current project patterns; the Playwright describe
callback and iterable callback-return rules are excluded because current test
and bridge patterns are intentional. `noNonNullAssertion` is excluded from
Biome while strict TypeScript (`strict`, `noUncheckedIndexedAccess`, and
`exactOptionalPropertyTypes`) remains enabled. These exceptions do not disable
the retained ESLint gate or browser/accessibility tests.

The developer scripts keep the existing entry points: `lint` runs Biome and
blocking ESLint, `format` and `format:check` use Biome, and `check` runs
typecheck, both lint gates, formatting, tooling contract tests, Vitest, and the
production web build.

# PR #687 — Catalog integrity and Global Search refinement
Status: ACTIVE work order; leader-owned. WP1 active; WP2 queued; WP3 leader acceptance only.
Baseline: `2e335f152fd98a085668647186e41c7e94e1dcfe`. Branch: `curation/ten-piano-sheets`. Existing draft PR: [#687](https://github.com/pumni/Sky-Auto-Player/pull/687). No other branch or PR. This order is implementation scope, not merge/release authorization.

## 1. Authority and working protocol

- Engineering Leader (ChatGPT): owns scope, technical decisions, acceptance criteria, sequencing, reviews and follow-up dispositions. Owner alone authorizes merge/release, distribution rights, physical-input/ReceiveOnly tests and credentials.
- Implementation Agent (local CLI): reads AGENTS.md, this order, active issue and PR comments at current HEAD; implements complete assigned package, tests, formats, captures evidence, pushes ordinary commits to this branch. Routine local implementation is delegated; no important architectural, product, data identity, security, contract or scope decisions. Stop and report a material ambiguity, unexpected source drift, failing independent oracle, or inability to meet an acceptance criterion. Never silently substitute a weaker design.
- Fetch branch, confirm clean worktree/current remote HEAD, inspect scoped instructions; do not discard owner changes. No new branch, PR, rebase, force-push, merge, release, workflow bypass, unrelated cleanup, dependencies or toolchain upgrades. Do not edit this work order or change issue status, close issues, or self-approve. No credentials/physical keyboard dispatch.
- Each WP is an end-to-end coherent delivery (not a few-file microtask). Commit coherent changes; avoid intermediate pushes of knowingly incomplete work. After local checks push the package once, even if hosted CI is still running/red. If a local check is blocked, disclose exact failure and push only coherent safe work, explicitly marked incomplete. Never claim skipped checks passed.
- Post handoff on the coordination issue and link it from PR if needed. Wait for explicit leader disposition before starting next WP, NOT for hosted CI green. Leader reviews code/evidence immediately on push; hosted CI is parallel signal, required only for final merge readiness. A review acceptance is bound to SHA and scope, not a blanket approval.
- Every replacement includes migration/removal of superseded classes, selectors, aliases, helpers, unused imports, unreachable branches, old assertions, stale fixtures/docs and obsolete tests. Search all callers and references (rg), explain retained compatibility with evidence. No parallel old/new path, deprecated fallback, dead code, TODO workaround or broad suppression. Do not delete unrelated public/wire contracts. If no obsolete path exists, explicitly report the audit.
- Work order is temporary. After accepted work, leader archives via immutable Git reference in docs/HISTORY.md per docs/AGENTS.md and removes this transient document from active tree; CLI does not do this without instruction. Store screenshots/logs in ignored test-results or PR artifacts, not proliferating permanent evidence folders.

## 2. Evidence and fixed decisions

CI [45987298817](https://github.com/pumni/Sky-Auto-Player/actions/runs/45987298817), static job [140466498490](https://github.com/pumni/Sky-Auto-Player/actions/runs/45987298817/job/140466498490): 325 pass, 1 fail, 3 ignored. Test committed_active_catalog_is_verified_from_the_live_manifest fails at checksum of sheets/3107.json: expected 226d4f8240a373a304fc0e9f494ac8d638dda5e897eae23a96a57b9e9be7ea5f, actual c75e76e0bcb0318d84f2d40ef05e46bd0ae9967f340a2a796f9210c6a4d8eb81. Source and pinned upstream 3107 blob both 421152985a5d886f04d11ecd8a80a8a9cdf3a0c5, 53,782 bytes, no CRLF. Leader recomputed 21/41 new files; all 16 batch-3 mismatched; five samples from earlier batches matched. Do not infer unchecked files are valid. Prior [run 45977258687](https://github.com/pumni/Sky-Auto-Player/actions/runs/45977258687) failed the same test at hardcoded count 137 vs 112. Current draft has 153 active entries (112 + 41). The downstream jobs were skipped because static gate failed.
  
UI: GlobalSearch.tsx combines type="search" (Chromium native cancel affordance) and custom Clear search button. Double X is strongly source-supported, not yet confirmed on packaged Windows. Current wrapper border-input maps Graphite #898c90 against #121314/#191a1b (5.51/5.16:1); ordinary border #2a2b2c fails 3:1. Existing theme-accessibility.spec.ts explicitly requires --input and >=3:1. Old repo screenshots (Nov 25, browser mock bridge) show bright idle border but are not current desktop tree or native evidence. User's own screenshot not attached at review time.

## 3. WP1 — Catalog integrity and CI recovery (ACTIVE)

Outcome: reproducible catalog source/manifest/inventory agreement, correct growth-aware test and no changes to UI in this WP.

### Implementation

1. Compare base/main at PR merge base with HEAD; record baseline active/retired counts and stable ID/path/title mapping. Confirm 41 added sheets and expected 153 active entries; no changed old sheet bytes/IDs, retired ID ledger unchanged. Stop if counts or provenance drift; do not change arrangement selections or invent credits.
2. For all 41 newly added sheets, use the exact upstream revision/path in docs/song-curation/piano-expansion-review.md (Ai-Vonie/Sky1984-Sheets-Collection at fdf69c6ed0ba8e9804a03e3e240eec31fb3056ea). Verify actual Git blob identity and exact bytes from pinned upstream vs committed branch and checkout. Compute SHA-256 independently from bytes (e.g. PowerShell Get-FileHash, sha256sum, Node crypto; no JSON serialization, newline/BOM normalization). Verify Git attributes and checkout bytes match Git blob; no weakening .gitattributes. Produce table path, upstream blob, local blob, upstream/local digest, old manifest digest and match result for all 41. If any source differs, stop and ask leader; don't blindly bless it. Audit every active manifest entry (153) and report all mismatches before refresh, not merely first fail-fast result. No need to commit a duplicate checker or one-off tooling.
3. Use pinned official command `node scripts/toolchain-pin.mjs exec -- cargo xtask builtin-catalog refresh` only after provenance check. It updates digests, validates sources. Review diff: preserve every existing active/retired ID, title, path, schema, order/ledger as far as canonical writer permits; checksum-only content changes. If pre-existing entries require changes or writer rewrites unrelated data, report and escalate. Do not change sheet files, reserialize JSON, change decoder, weaken checksum checks or add a second catalog owner.
4. In rust/xtask/src/builtin_catalog.rs, remove stale fixed 112 assertion from live manifest verification. Leader decision: manifest is authoritative (docs/library-v1-contract.md), not an exact count in generic code. Keep strict verified_sources success and cardinality equality; add explicit non-empty committed-catalog assertion to prevent vacuous pass. Keep existing independent fixture growth/source-drift, wrong hash/strict decoder, active/retired identity and other negative tests intact. Add focused regression only if uncovered (including empty live contract if needed; do not impose a new generic manifest API restriction). Expected 153 is a *this PR* acceptance check independently derived from base + 41, not a new production/test hardcode. Search other stale 112/137 count assumptions and fix only those representing live catalog, retaining revision-bound historical references.
5. Regenerate inventory via `node scripts/toolchain-pin.mjs exec -- cargo xtask song-audit inventory`; all three canonical files inventory.json, inventory.csv and inventory-summary.md under docs/song-curation. Verify 153 entries and exact manifest IDs/paths/digests; inspect strict parser/planner/Prepare diagnostics for new arrangements; report warnings/risk, no automatic musical selection or timing-policy changes. Repeat generation and compare byte-identical outputs. Update curation review SHA-256 tables and truthful validation status; don't claim listening or legal approval. Keep draft/release restriction. If any planner/parser issue needs a new policy or arrangement change, report separately, do not mask.
6. Run formatting/checks with pinned Rust/Bun toolchains. From repo root:
   - `node scripts/toolchain-pin.mjs exec -- cargo xtask builtin-catalog verify`
   - `node scripts/toolchain-pin.mjs exec -- cargo test --manifest-path rust/Cargo.toml -p xtask builtin_catalog::tests` (confirm package name/manifest and adjust only equivalent invocation according to actual workspace; report exact command)
   - `node scripts/toolchain-pin.mjs exec -- cargo fmt --manifest-path rust/Cargo.toml --all -- --check`
   - `node scripts/toolchain-pin.mjs exec -- cargo xtask check all` per owning CI/tooling docs; final pre-review check on clean commit if feasible; disclose blocked/skipped lanes, no fake pass.
   - `git diff --check` and status/diff scope audit. Check working-tree classification using canonical command if relevant, no changes to workflow to force skips.

### WP1 acceptance / handoff

- 41/41 provenance matches; 153/153 manifest/checkout digest matches; base + 41 IDs and retired ledger verified; all strict decode/verify checks pass; source sheets unchanged.
- Test no longer hardcodes count; real manifest still strictly validated; negative controls intact; inventory three files deterministic and 153 active; diagnostics reviewed. Curation report reconciled. No new ad hoc tooling or old hardcoded count path.
- List baseline/current SHA, commits, files, digest audit attachment, actual commands and exit codes, skipped/blocked reasons, inventory totals, planner/Prepare warning summary, dead-code audit and explicit non-claims (rights/listening/native safety).
- Push and report **WP1 READY FOR LEADER REVIEW**. Do not start WP2 until directed. Leader reviews immediately regardless of hosted CI status.

## 4. WP2 — Global Search UI, behavior and regression evidence (QUEUED)

Outcome: one clear affordance, quieter idle boundary, stable interaction/keyboard/focus across desktop themes/sizes without backend/architecture migration. Use current Base UI, Tailwind, Zustand and Tauri boundary. No new library, broad refactor or global input restyling.

### Design decisions (not delegated)

- One semantic owner for the composite search: keep native type="search" + accessible scope label/searchbox role, custom Button/X and existing search store. Disable browser cancel affordance only within [data-slot='global-search'] input[type='search'] via ::-webkit-search-cancel-button { -webkit-appearance: none; appearance: none; display: none; } in owning globals.css, scoped and documented. Do not hide all browser pseudo controls globally; no alternative text input path. Remove any superseded search-cancel styling if found. Inspect Windows WebView2 actual behavior; if not reproduced, report environment, do not invent a pass.
- Keep existing 32px title-bar geometry, spacing, current scope and slash hint rules, drag boundaries, keyboard shortcuts, current placeholder/ARIA, 120ms typed debounce, no layout redesign. Keep focus-within ring/offset and accessible keyboard focus for clear button. No native focus guard changes. Only custom button visible when draft has content; empty hides it. Click or keyboard activation clears immediately in UI, restores focus to actual input, cancels queued obsolete draft, eventual committed query empty and results restored; stale acknowledgements must not resurrect query or overwrite newer draft. Preserve selection, scope switching, external query updates and error reporting. Verify handling with and without optional inputRef. Use existing state/store request authority; correct narrow proven defects, don't build a second query scheduler. If the current ownership cannot meet these invariants without a larger redesign, stop for leader decision.
- Add dedicated semantic token `--chrome-input-border` and Tailwind @theme inline mapping `--color-chrome-input-border`, use `border-chrome-input-border` for outer GlobalSearch wrapper only; no hardcoded component colors. Set exact leader-selected values:
  
| Mode | Token | Contrast vs surface / card (computed from current tokens) |
| --- | --- | --- |
| Graphite Dark | #66696d | 3.37 / 3.16 |
| Aurora Dark | #586984 | 3.32 / 3.23 |
| Minimalist Dark | #69727d | 3.43 / 3.23 |
| Slate Dark | #5a7384 | 3.33 / 3.21 |
| Cyberpunk Dark | #865a86 | 3.31 / 3.21 |
| Classic Dark | #796a58 | 3.35 / 3.20 |
| Light (independent of saved palette) | #888a90 | 3.45 / 3.22 |
| Forced colors | ButtonText (or existing --input alias which resolves ButtonText) | system-controlled |

- Keep `--input`, existing 47-role palette baselines, other controls/dialogs/segmented inputs, ring colors, hover/active tokens and surfaces unchanged. Idle/hover use new border; focused uses existing ring. Test actual rendered adjacent chrome and input contrast >=3:1, text >=4.5:1, focus visible. Other dark titlebar uses card, Graphite/Light transparent over card. No weaker test or hardcoded magic CSS overrides. Update existing search-specific oracle to new semantic owner; independent explicit palette expectation and rendered contrast remain. If token/cascade values fail actual rendering, return evidence and ask leader rather than choose another color silently.
- Update owning docs/desktop-workbench.md to explain narrow search boundary token and behavior; no second design-system document. Remove replaced search-specific border-input expectation, do not change generic input assertions.

### Tests and visual verification

- Unit: extend GlobalSearch.test.tsx for exactly one custom accessible clear button when populated/none when empty; clear restores focus, immediate draft clear + eventual store empty; debounce cancellation before request, clear during older in-flight query incl. old committed query already empty, stale ack after clear/new draft, external scope changes; retain current tests (searchbox semantics, scope, focus, async ordering). Use real store/mock bridge and controlled promises as appropriate; do not write tests that pass without actual behavior.
- Browser: extend theme-accessibility.spec.ts with token resolution/actual rendered boundary contrast for six dark palettes + shared Light and forced-colors focus; keep existing baseline/contrast checks. Add focused search interaction regression in appropriate existing titlebar/library suite: type, click/keyboard clear, assert searchbox focus and query/results, slash shortcut, verify custom button count and native pseudo suppression using CSS/visual evidence (DOM count alone cannot observe UA shadow control). Test empty, populated focused, populated blurred, clear/hover; widths 1366x768 and 800x560, long scope and no clipping or drag regression. Retain existing titlebar geometry assertions.
- Capture before/after at same viewport/palette/content for Graphite, Light and compact, include populated focused state (clear X), empty unfocused and focused keyboard state, optional palette contact sheet. Browser mock evidence labelled as such. Capture actual Windows Tauri/WebView2 screenshots in safe UI-only session, record WebView2 version/build source and theme, no physical input dispatch, no gameplay/ReceiveOnly, no release/install authorization implied. If native cannot run, document blocker and preserve browser proof; leader decides native qualification. No new permanent screenshot directories. Use existing `bun run ui:refresh` only if updating canonical visuals per docs/ui/README.md, not as a substitute for typed-state evidence. Ensure final capture source/desktop tree identity; do not claim stale captures current.
- Focused commands in apps/desktop: `bun run test -- src/components/chrome/GlobalSearch.test.tsx`; `bun run test:e2e -- tests/e2e/theme-accessibility.spec.ts tests/e2e/titlebar.spec.ts` plus affected library/browser suite; use actual script argument convention and report exact commands. Run `bun run typecheck`, `bun run lint`, `bun run format:check`, `bun run check` (includes all local desktop Vitest/build/tooling checks). Format only touched files with owning formatter; no repo-wide unrelated formatting. From repo root run pinned `cargo xtask check all` final on clean commit. No synthetic performance benchmark mandated for CSS-only change; if async flow changed, report measured typing-to-query/clear behavior vs baseline with repeatable scenario, 120ms debounce preserved and no duplicate requests. No arbitrary latency SLA or fabricated measurement.

### WP2 acceptance / handoff

- Exactly one visible X in real rendered search when nonempty, none when empty, accessible semantics retained, stale request/clear test passes; native limitation explicitly flagged if unresolved.
- New token only owns search boundary; all seven appearances and forced colors meet specified behavior. No altered global input palette or generic tests, no lost focus, shortcut, titlebar geometry or search functionality. Evidence shows idle border visibly calmer vs before and focus remains clear.
- No extra CSS selectors, duplicate clear paths, old search-only border expectation, unused imports, dead helper or duplicate fixtures left. No architecture drift.
- Post **WP2 READY FOR LEADER REVIEW**, exact SHA, test results, screenshots/artifact links with environment/source identity, diff and removal audit. Push coherent package without waiting for hosted CI. Stop.

## 5. WP3 — Leader integration/acceptance (not CLI autonomous)

Leader reviews each pushed diff against last accepted SHA and against PR base, reads touched source/tests/owning contracts and actual CI logs (no reliance on CLI summary alone). Review independent evidence, regression controls, security/architecture boundaries, scope creep, stale/unused code and docs. Classify findings blocker/major/minor, give explicit fixes and acceptance tests; CLI addresses grouped findings on same branch and resubmits. Review can overlap running/failed CI. Record separate dispositions: WP code accepted / changes requested / blocked, local checks, hosted CI, native visual evidence, rights/listening.

Final gate: all required relevant CI jobs pass for exact PR HEAD; pinned clean-source check all; 153 active verified, all accepted WP outcomes, no unresolved blocking findings, no unrelated diffs or old architecture. Archive temporary order only on leader instruction; any final docs-only commit still requires exact-head CI evidence. Existing draft PR remains draft unless owner authorizes readiness. No merge/release until independent listening/rights and any required native/package qualification are authorized and satisfied. Technical acceptance is not permission to distribute compositions.

## 6. Standard handoff comment template

```text
WP: WP1 / WP2
Status: READY FOR LEADER REVIEW / BLOCKED
Branch / PR:
Starting SHA / pushed HEAD:
Commit(s) / changed files:
Implemented and decisions followed:
Tests (exact command | exit | result | duration):
Evidence (source SHA, environment, links; WP1 provenance/inventory or WP2 screenshots):
Dead-code / obsolete-path audit (removed / retained + why):
Unexpected findings, skips, local blockers, CI run/status (do not wait):
Non-claims / open decisions:
```

Leader will publish acceptance or a bounded corrective package on GitHub. Implementation agent must not treat silence, green CI or a successful build as approval.

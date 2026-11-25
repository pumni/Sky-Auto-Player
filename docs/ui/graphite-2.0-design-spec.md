# Sky Auto Player — Graphite 2.0 UI/UX & Motion Design Specification

**Revision:** 1.1 — Proposed / supersedes v1.0 visual direction  
**Design direction:** Clean Desktop Workbench — VS Code 2026 inspired  
**Platform:** Windows desktop, Tauri/WebView2  
**Reference:** main `170ccd22fc0a94171da8429dacb2b526a72a797d`  
**Language:** Vietnamese design rationale; English product copy examples.  
**Scope:** Graphite default Dark and shared Light appearance, equal priority; shared UI behavior; no repository writes.

## 0. Status and evidence boundary

- Source: https://github.com/pumni/Sky-Auto-Player/tree/170ccd22fc0a94171da8429dacb2b526a72a797d/apps/desktop
- Visual evidence: https://github.com/pumni/Sky-Auto-Player/tree/main/docs/ui/current
- Manifest source `0a7e81a5f0322d39ca952ea09ea4b6253385d0e8`, desktop tree `1443a2b6ec4aa45a9ae38151f46ee73f6d30bc89`; HEAD desktop tree `155e9e999f4f072e81cb15fb224347c967088ff5`.
- Compare: https://github.com/pumni/Sky-Auto-Player/compare/0a7e81a5f0322d39ca952ea09ea4b6253385d0e8...170ccd22fc0a94171da8429dacb2b526a72a797d . Three commits ahead; only desktop file changed is `tests/e2e/design-system-finishing.spec.ts`; no production desktop source changes in that comparison. Nonetheless manifest fails the repository's tree identity rule. Refresh before formal visual acceptance.
- Browser mock screenshots are not native qualification. No native build, test suite, usability study or profiling was executed for this specification.
- MUST = proposed acceptance requirement once approved; SHOULD = design recommendation; MAY = optional experiment. Numeric values are project design proposals, not external platform mandates.

## 1. Product principles and non-goals

1. Music-first: the selected sheet and current playback are separate concepts, and the user can tell them apart without color alone.
2. Clean Desktop Workbench: neutral graphite surfaces, thin seams, restrained periwinkle/indigo accent, compact controls and consistent geometry in both appearances. Dark content is slightly darker than surrounding chrome; Light content is slightly lighter. Avoid glass, neon, gradients, fake album art, decorative tiles and looping motion.
3. Truthful state: native engine and existing store contracts own playback state; no optimistic claim that input was sent, stopped or successful. Pending, acknowledged, terminal and recovery states must remain distinct.
4. Desktop-first: keyboard, focus, resize, DPI, scroll position and persistent pane preferences are core behavior, not polish.
5. Progressive disclosure: present summary and next action before technical detail, retaining all existing details and debug affordances.
6. CSS-first: retain Base UI, Tailwind, React, Zustand, virtualization, bridge and Rust boundaries. No new motion dependency required.

Non-goals: no seek/volume/audio waveform/queue unless supported by existing domain; no automatic change of playback profile, timing, admission decision or physical input behavior; no changes to the effective colors of the five other dark palettes; no Tauri window minimum changes; no Figma file or production implementation at this stage. Light is an explicit deliverable; Follow Windows and all other dark palettes must retain their existing behavior. Do not introduce a new persisted light-palette field or rename palette IDs. Maintain current UI language unless localization is separately approved.

## 2. Foundation tokens

### 2.1 Research and design rationale (v1.1)

User reference: uploaded VS Code screenshot, 1920×1080. Exact frequent RGB samples from the supplied PNG: #121314 (1,086,807 pixels), #191A1B (810,055), #202122 (46,330), #297AA0 (19,100), #2A2B2C (11,753). Pixel counts include all UI, text and antialiasing; they do not establish the exact installed VS Code version or user settings. These samples correspond to roles in the published 2026 Dark theme, but the screenshot is the visual authority for the requested style. Microsoft’s Oct 15 2026 modernization article emphasizes a narrow surface range, coherent shared shell, less visual noise, connected rather than pill tabs, density and repeatable performance measurements. Published 2026 Light uses white editor and near-white surrounding chrome. Borrow this surface relationship, not VS Code’s brand blue, IDE menus, icons or layout features. Fluent semantic aliases support separate values for both modes. WCAG distinguishes decorative borders from essential control/focus cues.

### 2.2 Dual appearance palette — candidate v1.1

Keep existing primary action tokens: the repo's `graphiteThemeTokens.test.ts` explicitly pins approved indigo actions in both modes. This is a minimal-delta refinement, not a rebrand. The current `resolveAppearance` retains saved dark palette even when mode is Light, and `applyAppearance` writes `data-palette` and `data-color-mode`. The global Light block already ignores the dark palette. Preserve this architecture: “Graphite Light” is design terminology for the shared Light appearance, not a new palette ID. Follow Windows resolves effective mode; saved non-Graphite dark palette remains untouched and restored on Dark. No native DTO/IPC/storage changes. 

All existing semantic color roles are listed below. Values are opaque sRGB candidates; no raw HEX in product components. Apply Graphite-specific overrides after dark palette rules and before forced-colors; apply shared Light for all saved palettes. Preserve existing root fallbacks for the other five dark palettes (including inherited semantic roles, not merely explicit overrides). Use selectors or equivalent cascade verified by computed style tests; do not blindly change :root or change specificity in a way that overrides forced colors. Keep structural geometry shared across modes/palettes. If literal-token tests require migration, explicitly review the changed approved roles and update tests to check effective styles, not weaken thresholds.

| Existing role | Dark | Light | Notes |
|---|---|---|---|
| `background` | #121314 | #FFFFFF | outer content canvas / startup |
| `surface` | #121314 | #FFFFFF | track browser canvas |
| `card` | #191A1B | #F7F7F8 | chrome, navigator, inspector, player |
| `popover` | #202122 | #FFFFFF | elevated surfaces |
| `foreground` | #E2E4E8 | #25272A | Semantic role retained |
| `text-secondary` | #B3B5B8 | #505359 | Semantic role retained |
| `text-tertiary` | #A4A6AA | #5D6066 | Semantic role retained |
| `border` | #2A2B2C | #DCDDE0 | decorative seam |
| `overlay-border` | #454749 | #C4C6CA | overlay edge, decorative |
| `input` | #898C90 | #75787E | essential input outline |
| `primary` | #8E9DFF | #3C54CF | existing brand action |
| `primary-hover` | #A3B0FF | #3248BA | Semantic role retained |
| `primary-pressed` | #7888F0 | #293DA3 | Semantic role retained |
| `primary-foreground` | #11131A | #FFFFFF | Semantic role retained |
| `accent-ink` | #9AA8FF | #3449B5 | Semantic role retained |
| `ring` | #9AA8FF | #3449B5 | keyboard focus; offset from fill |
| `selection` | #303132 | #E4E5E7 | neutral selected row + marker |
| `playing` | #202326 | #EDF0F4 | quiet current-session surface + music icon |
| `row-hover` | #242526 | #F0F1F2 | Semantic role retained |
| `control-hover` | #303132 | #EDEEF0 | Semantic role retained |
| `control-pressed` | #393A3C | #E0E2E5 | Semantic role retained |
| `control-active` | #303132 | #E4E5E7 | Semantic role retained |
| `control-disabled` | #6C6E73 | #8A8D94 | Semantic role retained |
| `secondary` | #242526 | #EDEEF0 | Semantic role retained |
| `muted` | #242526 | #F0F1F2 | Semantic role retained |
| `accent` | #303132 | #E4E5E7 | Semantic role retained |
| `success` | #64C496 | #1F7A47 | Semantic role retained |
| `success-surface` | #143326 | #E9F5ED | Semantic role retained |
| `success-border` | #489E74 | #1F7A47 | Semantic role retained |
| `warning` | #F5C451 | #8D5A08 | Semantic role retained |
| `warning-surface` | #3A2C11 | #FFF5DF | Semantic role retained |
| `warning-border` | #B4862D | #8D5A08 | Semantic role retained |
| `danger` | #F27A80 | #B4232F | Semantic role retained |
| `danger-surface` | #3A1C22 | #FCEBED | Semantic role retained |
| `danger-border` | #BE5962 | #B4232F | Semantic role retained |
| `destructive` | #C42B36 | #B4232F | Semantic role retained |
| `destructive-hover` | #D13A45 | #9F1F2A | Semantic role retained |
| `destructive-pressed` | #9E1F2A | #861A23 | Semantic role retained |
| `destructive-foreground` | #FFFFFF | #FFFFFF | Semantic role retained |
| `scrim` | #08090C | #16181B | modal backdrop; retain 55% initially |
| `scrollbar-thumb` | #777A7E | #85888E | visible thumb, 3:1 tested |
| `scrollbar-thumb-active` | #A4A6AA | #5D6066 | Semantic role retained |
| `card-foreground` | #E2E4E8 | #25272A | Semantic role retained |
| `popover-foreground` | #E2E4E8 | #25272A | Semantic role retained |
| `secondary-foreground` | #E2E4E8 | #25272A | Semantic role retained |
| `accent-foreground` | #E2E4E8 | #25272A | Semantic role retained |
| `muted-foreground` | #A4A6AA | #5D6066 | Semantic role retained |

Region aliases: `surface-chrome=card`, `surface-navigation=card`, `surface-content=surface`, `surface-inspector=card`, `surface-player=card`, `surface-elevated=popover`. Use neutral selection fill; distinguish selected with leading marker and selection semantics, playing with music/session icon and accessible status. Do not make every title, icon or active control indigo. Accent reserved for primary action, focused/active cues, current session and progress. Use semantic colors only for actual statuses. Normal readiness inline with text/icon, not a large filled green card. Essential control borders use `input`; `border` and `overlay-border` may be lower contrast if decorative. Pane resize target remains separate from its visible stroke. Light elevation: subtle edge plus localized shadow for popup only; Dark: slightly raised fill plus edge. No pane shadows or blur.

#### Contrast qualification (computed token samples)

The matrix evaluates foreground, secondary, tertiary and accent ink across 13 neutral/interactive surfaces; focus and input outline across the same surfaces; primary and destructive foreground against three action states each; three status text/surface pairs; scrollbar thumb on three backgrounds. This is a candidate-level calculation, not a rendered audit. Compare full-precision values to threshold; sample displayed rounded values only. 90 checks per mode. Additional rendered checks (alpha compositing, status borders where essential, disabled behavior, focus offset against actual adjacent colors, charts and native chrome) remain mandatory.

| Pair / minimum across neutral surfaces | Dark | Light | Threshold |
|---|---:|---:|---:|
| foreground / worst neutral | 8.94 | 11.54 | 4.5 |
| text-secondary / worst neutral | 5.54 | 5.94 | 4.5 |
| text-tertiary / worst neutral | 4.67 | 4.86 | 4.5 |
| accent-ink / worst neutral | 5.11 | 5.85 | 4.5 |
| ring / worst neutral | 5.11 | 5.85 | 3.0 |
| input / worst neutral | 3.37 | 3.41 | 3.0 |
| primary-foreground / primary | 7.44 | 6.25 | 4.5 |
| primary-foreground / primary-hover | 9.02 | 7.58 | 4.5 |
| primary-foreground / primary-pressed | 5.81 | 9.17 | 4.5 |
| destructive-foreground / destructive | 5.61 | 6.52 | 4.5 |
| destructive-foreground / destructive-hover | 4.77 | 7.77 | 4.5 |
| destructive-foreground / destructive-pressed | 7.83 | 9.61 | 4.5 |
| success / success-surface | 6.45 | 4.77 | 4.5 |
| warning / warning-surface | 8.33 | 5.39 | 4.5 |
| danger / danger-surface | 5.75 | 5.66 | 4.5 |
| scrollbar-thumb / surface | 4.31 | 3.55 | 3.0 |
| scrollbar-thumb / card | 4.04 | 3.32 | 3.0 |
| scrollbar-thumb / popover | 3.74 | 3.55 | 3.0 |

Preserve first paint: synchronize startup CSS explicit dark/light and system fallback, native Tauri window background/reveal, brand asset variants, Settings preview swatches and runtime effective roles. Existing startup tests pin correspondence to root and light block: migrate deliberately if scoped default aliases change, test real computed styles for both modes before/after bootstrap, and retain all other dark palette colors. No app-wide crossfade or white/dark flash during startup or switching.

### 2.3 Typography

Segoe UI Variable → Segoe UI → system-ui; current monospace for raw diagnostics. Use tabular numerals for duration, page number and numeric readouts. No external fonts.

| Role | px / line height | Weight |
|---|---|---|
| Dialog title | 24 / 32 | 600 |
| Library heading wide / compact | 20 / 28; 16 / 22 | 600 |
| Selected song title | 18 / 24 | 600 |
| Section header | 14 / 20 | 600 |
| Track title / main label | 14 / 20 | 400 or 500 |
| Compact track title | 13 / 20 | 500 |
| Metadata | 12 / 18 | 400 |
| Column heading / overline | 11–12 / 16 | 500–600 |
| Numeric readout | 12–13 / 18 | 400–600 |

Use foreground for idle track titles, secondary for metadata. Use sentence case for section labels, navigation and table headers; no tracking-wide all-caps decoration. Avoid uppercasing body copy. One-line title truncation must preserve full accessible name and existing tooltip behavior; dialogs and error reasons wrap. No font-weight changes that cause table columns or transport positions to shift.

### 2.4 Spacing, shape, elevation

Spacing scale 4, 8, 12, 16, 20, 24, 32 CSS px. Exceptions (e.g. 46px row, 2px ring, existing geometry) documented rather than forcibly rounded. Candidate radii 6 control, 8 pane, 8–10 popover, 10–12 dialog; keep existing radii on shared primitives until tested across all palettes. circular only for icon/transport or status dot. Typical icon glyph 16px, main transport 18–20px, secondary transport 16–18px. Hit area independent of glyph, typical 32–40px. Layout gap 4px at workbench separators; section padding 12–16px; dialog 20–24px. Use a consistent 4px shell gap and subtle 1px pane seams (not no-border cards). Use no shadow for normal panes, restrained existing shadow for popovers and dialog; no blurred background filter. Keep scrim 55% initially; validate foreground/background contrast of overlays.

## 3. Layout contract

Native initial size 1200×760, minimum 800×560 from `src-tauri/tauri.conf.json`. Browser-only stress cases 640×448 and 640×320 remain tests, not supported native sizes. Preserve `workbenchGeometry.ts`, persisted preferences, minimum track width 420px, overlay width 288–352px and existing collapse/docking rules. Do not create a competing set of CSS breakpoints. Default geometry remains current unless a measured proposal is approved. Review both user-expanded and auto-collapsed navigator at 800px. Do not force a floating dock on compact; retain full-width anchored dock, top separation and existing available height. Candidate mock layout: title bar 40, center workbench fills available area, dock about 92px; preserve exact current measurements as baseline and adjust only with tests. Window drag region and caption controls unaffected.

| Case | Expected outcome |
|---|---|
| 1920×1080 | spacious workbench, no wasteful card proliferation |
| 1366×768 / 1200×760 | standard workbench, library + content + inspector when geometry permits |
| 1024×640 | existing pane policy, table container queries hide lower-priority metadata |
| 800×560 | compact native; no hidden primary actions, profile and settings fit |
| 640×448 / 640×320 | browser-only, overlay and scroll robustness; no unintended horizontal overflow |
| 125%,150%,175%,200% DPI | native titlebar hit tests, focus, text truncation, popover clipping, resizing |

## 4. Component specifications

### G2-C01 — Chrome and search

Files: `components/chrome/AppTitleBar.tsx`, `GlobalSearch.tsx`, `WindowCaptionControls.tsx`.
- Keep height 40px, current center search width behavior, system caption controls at right; preserve draggable free area. Search target ~32px; icons 34px target with visible focus.
- Scope label noninteractive unless real scope selector exists. Example placeholder `Search built-in songs…`, `Search this playlist…`; do not imply cross-library search when source-local. Preserve `/` shortcut if currently supported; do not add unsupported shortcuts without mapping and tests.
- Clear action retains focus to search. Empty results show query and clear action. Reload and update show real pending/result feedback, version tooltip where data is available. Do not indicate success before acknowledgement.
- AC: search typing, clear, shortcut, scope, long query, keyboard focus and native drag/maximize/snap unchanged; no caption animation overrides.

### G2-C02 — Library navigation

Files: `LibraryNavigator.tsx`, `LibraryNavItem.tsx`, `PlaylistNavItem.tsx`, `LibraryAddMenu.tsx`, playlist dialogs.
- Expanded item height ~40–42px; compact 40px target; existing 3px accent indicator. Active item selected surface + marker + aria-current; idle neutral, hover controlled. Counts 12px tabular, names ellipsis with full tooltip/name.
- Empty state `No playlists yet` / `Create a playlist to organize your songs.` / `Create playlist`; use a neutral outlined secondary CTA; retain import path if available through menu. No marketing copy or illustration required.
- Keep create/rename/delete, empty/loading/error/retry states and persisted collapse preference. Delete must name actual playlist and accurately describe consequences (do not assume deletes songs).
- AC: keyboard access to every playlist and actions in expanded/collapsed mode, correct selection after rename/delete, errors recoverable, large library scrolls.

### G2-C03 — Track browser and table

Files: `TrackBrowser.tsx`, `TrackTable.tsx`, `TrackRow.tsx`, `TrackTableHeader.tsx`, `BuiltinPager.tsx`, `TrackActionsMenu.tsx`, `PlaylistAddSongsMenu.tsx`.
- Keep fixed virtual row height 46px (one shared constant/token for CSS and virtualizer estimate/fallback). No density setting this release. Header heading 20/28 when wide, 16/22 compact; keep current header/columns geometry. Primary title foreground; notes and duration secondary; maintain current container-query column hiding and numeric alignment.
- Row state matrix: idle=neutral; hover=row-hover; selected=neutral selection + 2px accent leading indicator; playing=playing + music icon + accent title; both=selection + both indicator and music icon; paused=current-session cue distinct from active playback if backed by native state. Focus=independent 2px ring on all interactive states. Never rely on only color to distinguish selected and playing. Do not add second line or change row height for status.
- Keep virtualized DOM, paging and keyboard navigation semantics. No stagger, FLIP, row height animation, or animated filtering. Search/paging preserve correct selection and scroll policy; locate uses existing behavior. `aria-selected`, `aria-current` retain correct mapping.
- Like/More actions remain visible on focus, selected, playing; keep liked icon visible. Experiment with permanently visible neutral Like only after reviewing density and usability; default preserve current hidden-on-idle policy. If opacity animates, also verify pointer hit behavior and keyboard focus; hidden should not mislead pointer users.
- Footer retains locate + count + pager; consistent tooltips and accessible names. Keep loading/no-results/import failure/catalog unavailable states distinct. Prefer stable skeleton only when loading is perceptible; no shimmer.
- AC: selected Blue Bird while Candle Run is playing; both same; pause/stop; hover/focus; keyboard cross-page; long Unicode titles; liked/unliked; pending/empty/error; large catalog resize without jumps or lost anchor.

### G2-C04 — Player dock and transport

Files: `PlayerBar.tsx`, `PlayerTransport.tsx`, `PlayerTrackInfo.tsx`, `PlayerTools.tsx`.
- Three fixed logical zones: track identity, centered transport/timeline, tools. Primary circle 42px baseline, 44px candidate only if all geometry tests pass; retain 34px secondary target and 32px tools initially. Play/Pause center remains invariant across state; no shifting button group. Stop remains directly accessible when engine allows it. Keep shuffle, previous, next, autoplay, profile and inspector.
- Name 14/20 semibold; status 12/18 with text and icon where useful. Active session identity overrides selected track in dock according to existing store contract. No track artwork dependency and no decorative tile; retain simple existing music glyph.
- Timeline nonseekable native progress, 4px baseline or 5–6px candidate with geometry tests. No draggable thumb, slider semantics, click-to-seek, volume or waveform. Tabular elapsed/total. Unknown duration and pending status do not invent time.
- Canonical wording `Test mode` / `No keyboard input` as space permits; compact `Test mode` with tooltip; diagnostics can use `Dry run (Test mode)` on first mention. Only assert no input when session mode confirms it. Use current native mode and bridge flags, not inferred color/icon.
- State presentation matrix (map to existing enums; no domain changes): idle no selection→disabled Play if unavailable; selected→Play only if admitted by existing logic; preparing/starting→pending icon and status; awaiting admission→decision panel; active→Pause/Stop according to existing capabilities; paused→Resume; focus issue→reason/action as permitted; stopping→pending until confirmed; terminal→finished/stopped/error with existing recovery. Do not remap native focus state to paused unless state contract says so.
- AC: existing transport geometry tests, rapid play/pause/resume/stop, conflicting command, late events, changed selection, dry run, focus loss/recovery, error, long titles at 800×560; no stale session or misleading optimistic state. Icon crossfade never gates command dispatch.

### G2-C05 — Playback profile

Files: `PlayerTools.tsx`, `TimingMarginControl.tsx`, formatting utilities.
- Preserve existing 360px maximum popover, 320px minimum where fits, anchor and scroll handling. Basic two columns: Base Hold and Tempo; explain frame units using backend-derived hint. Advanced Timing collapsible; FPS, margin, readouts. Use `Target FPS` only if semantics match contract. Existing native allowed options/units remain authoritative. No new slider range, no duplicate frontend timing calculations for runtime decisions.
- Readout group with tabular numbers; highlight effective target hold. Safe Preview text clearly states no keyboard input, and action remains disabled when unavailable. Reset to default only if native/store support it and separate approval obtained. Popover should not cover Stop while active if that compromises control access; test geometry and policy first.
- AC: existing VQC expanded/select 800×560, keyboard select, nested popover, pending validation, escape/focus return, no viewport overflow, unchanged numeric values/commands.

### G2-C06 — Inspector details / runtime

Files: `UtilityPane.tsx`, `UtilityHeader.tsx`, `SongDetailsView.tsx`, `DiagnosticsView.tsx`, `Diagnostics*`.
- Docked pane neutral baseline, 12–16px padding, header and tabs stable. Overlay uses existing position and scroll. Top section label `Selected song`, title and metadata; status first. `Sheet ready` / `Sheet checks passed` only for actual sheet validation; explicitly say target and focus checked when starting, if true. Preserve compatibility, timing risk and all rationale. Use icon + label + semantic status color. Default readiness is inline; only use larger warning surface when action required; don't turn every status into a card.
- Runtime summary session name, confirmed state, mode; retain Performance / Timing / Events. Unknown measurements `—` or `Not measured`, dry run reason. Keep raw event identifiers and session identifiers accessible; optional human-readable event label mapped from known event names, raw data copy/export remains. No invented metrics. Plot must label units and remain accessible without color. Export state/error feedback clear.
- AC: inspector follows selection while runtime follows active session; no selected/playing confusion; pane close/escape restores focus; keyboard tab switch; no-data, dry-run and populated states; diagnostics exports unchanged; responsive scroll.

### G2-C07 — Playback admission / recovery

Files: `PlaybackAdmission.tsx`, `TimingAdvisory.tsx`, related tests.
- Existing anchored admission panel, width ≤600 and scroll constraints preserved. Title + song + risk severity + plain-language reason + consequence + decision options + cancel. Do not conflate timing risk with account risk. Keep all native offered decision variants and unique explanations.
- Proposed CTA policy: if a risk exists and a dry-run choice is offered, test-first is preferred for novice UX, pending product approval; retain proceed option and explicit consequences. For low/no risk use normal existing flow. For other decisions do not force a test option that does not exist. Never silently select a decision or bind Enter to an unreviewed proceed action. Preserve engine validation and default policy until approved.
- Risk wording example (short same-key gap): `Some repeated notes may not be detected by the game.` Technical detail remains expandable. `Test playback (no input)` and `Proceed with current settings` remain distinct. Recovery banners state cause, current confirmed status and next available action. Escape/dismiss respects existing semantics; does not silently stop an active session.
- AC: medium/high/variant fixtures, narrow and long-title scenarios, keyboard focus, cancel no-start, test sends no physical input as verified natively, proceed only sends admitted decision, pending double-click cannot duplicate command. Error/retry and conflict status remain actionable.

### G2-C08 — Settings and appearance

Files: `SettingsPanel.tsx`, `SettingsCategoryContent.tsx`, `AutoPlaySwitch.tsx`, `DonateSettings.tsx`.
- Preserve max 820×640 dialog, 164px categories and independent scrolling as baseline. Header 24/32; section 14/20; setting labels 14/20; descriptions 12/18. Group basic and advanced only where semantically related. Always retain all six categories and keyboard access.
- Appearance: retain current select control initially; theme control `Follow Windows`, `Light`, `Dark` with clear selected state. Prefer miniature swatches only if 800×560 geometry passes; otherwise existing select with better preview. Palette choice separate from active mode; describe effective Light as `Light` and saved dark palette separately (Graphite Light is design terminology only). Theme persists per existing native logic. No changes to effective colors of other dark palettes; Light itself is updated. Preview both appearances with the actual semantic tokens. No app-wide theme crossfade in v1; immediate apply prevents flash and respects OS preference.
- Auto Play actual On/Off and pending/error state; no silent setting changes. Diagnostics descriptions accurate about collected data; updates retain channel and actions; About/Donate retain information and existing safe external link handling. No new data collection.
- AC: settings 800×560 and 640×320 browser scroll, category navigation, focus restore, settings persistence, failed save, all palettes and Light/Follow Windows, no clipped selected option or overlay.

### G2-C09 — Shared UI primitives and update

Files: `components/ui/{button,input,select,switch,tabs,tooltip,dialog,alert-dialog,popover,dropdown-menu,scroll-area,StatusBadge}.tsx`, `UpdateDialog.tsx`, `BootstrapGate.tsx`, `ErrorBoundary.tsx`.
- Controls 36px default, 32px compact, primary transport separate; focus ring visible and not clipped. Button states rest/hover/pressed/focus/disabled/busy. Busy reserves label geometry, no fake progress. Popups anchor to trigger and use existing Base UI collision handling; menu item 32px min, checkmarks and focus distinguish states. Preserve semantic dialog roles, labels and focus trap; don't introduce nested modal stacks without tested behavior.
- Tooltip delay current 350ms, close delay 80ms; enter/exit below. Only show on applicable hover/focus; Escape dismiss where primitive supports. Do not hide essential text solely in tooltip. Scrollbar existing reveal-on-scroll/drag policy retained; forced colors always visible; always-show preference optional later.
- Update dialog uses actual updater state; release notes scroll, footer accessible. Determinate progress only when total available, indeterminate otherwise. `Update and restart` explicit. Preserve existing native updater state machine and retry actions; no fake ETA. ErrorBoundary and bootstrap show recovery/instructions without indefinite animated splash.
- AC: primitives mouse/keyboard/focus/disabled/forced-colors/reduced motion; long text and nested popup; update available/downloading/error/retry/handoff fixtures, viewport and startup tests.

## 5. Motion contract

Preserve existing easing values: standard `(0.2,0,0,1)`, enter `(0.16,1,0.3,1)`, exit `(0.4,0,1,1)`. Duration aliases may reference existing 80/120/150/180/240ms primitives or explicit asymmetric semantic values. Do not rename existing tokens without migration. Prefer opacity/transform; small color transitions okay; animate tab indicator geometry only if profiling acceptable.

| ID | Trigger | Animated properties | Enter/exit or duration | Reduced motion |
|---|---|---|---|---|
| M01 | Button hover/pressed | background/color | 120/80ms | immediate |
| M02 | Track hover/selection | background, indicator opacity | 120ms | immediate |
| M03 | Like toggle | color/fill, optional local scale | 120ms | immediate |
| M04 | Play/Pause/pending icon | opacity on same fixed box | 100ms | immediate icon/state; static pending indication |
| M05 | Tabs | current indicator position/size, color | 150ms | snap |
| M06 | Tooltip | opacity | 100/80ms | immediate |
| M07 | Select/menu | opacity; optional scale .99→1 only for popup, anchored origin | 150/100ms | immediate |
| M08 | Popover | opacity; optional scale .99→1 only for popup, anchored origin | 180/120ms | immediate |
| M09 | Dialog | opacity + optional scale .99→1, scrim opacity | 200/140ms | immediate |
| M10 | Inspector overlay | opacity + translateX 8–12px→0 | 220/160ms | immediate |
| M11 | Pane collapse/expand | existing boundary interpolation, no row reflow animation | 180–240ms candidate, profile | snap |
| M12 | Pointer pane drag | geometry follows pointer | no easing | identical |
| M13 | Scrollbar | opacity | 80/180ms | immediate |
| M14 | Admission/recovery | optional short opacity (no delay of action) | 120ms | immediate |
| M15 | Theme apply | none in v1 | 0ms | 0ms |
| M16 | Timeline | real presentation data; no invented motion | native event cadence | same data; no decorative motion |

All animation is interruptible. New intent wins; reverse transition from current interpolated value if feasible; no animation queue. Closing overlay immediately stops pointer/focus interaction and restores focus correctly even while exit pixels remain. Handle reduced motion while panel is open and when transitions are cancelled; do not rely solely on `transitionend` for unmount/cleanup. UI command, state reconciliation, focus and error feedback are not delayed by animation. No decorative press bounce, no animated theme switching; keep state feedback short. No `transition: all`, no animated virtual rows, no global backdrop blur, no global `will-change`, no always-on progress glow. Pending spinner is bounded by actual pending state; reduced motion uses non-moving status with accessible text. If animation regresses playback timing, disable it before optimizing decorative details.

## 6. Accessibility & UX quality gates

Target WCAG 2.2 AA as a frontend benchmark, not a certification claim. Normal text ≥4.5, large ≥3, essential visual controls/states ≥3, minimum targets 24×24 CSS px subject to WCAG exceptions; desktop design targets mostly 32–40. Require visible keyboard focus and avoid obscuring focus (AA 2.4.11); aim stronger than minimum. Respect prefers-reduced-motion (2.3.3 is AAA, adopted as product requirement) and forced colors. Don't use color alone for selection, playing, risk, errors. Screen reader names, roles and state truthful; status changes announced once, not every timer tick. Verify Narrator/NVDA, keyboard tab order, Escape, focus return, disabled/pending interactions, text zoom/high DPI, long Unicode names and accessible scroll. Base UI semantics and existing documented focus guard adjudication remain; don't blanket-disable accessibility rules.

## 7. Performance and native qualification

Phase 0 establish same-machine baseline using current scripts and native tests; pin revision, build type, OS, WebView2 version, game version/mode, display refresh, DPI, catalog size and playback profile. Browser mock traces are separate from native evidence. Use existing profiling scripts `profile-builtin-library.mjs`, `profile-workbench-resize.mjs`, `benchmark-startup.mjs`; verify invocation in package.json. No arbitrary emitted-JS bundle ceiling (docs/ui/README.md D13). Collect baseline and candidate repetitions (proposed ≥5 per scenario) with median/p95 and variance; do not conclude based on one run. For UI target <100ms direct feedback and p95 frame intervals near 16.7ms on 60Hz under ordinary workload; distinguish dropped frames/long tasks and actual rendering from input-to-paint. Compare native sender timing, late events, missed/release events, focus safety, cancellation, memory/CPU under comparable workloads; thresholds must derive from existing native contract and agreed baseline variability, not invented millisecond limits. No timing regressions beyond accepted noise, no incorrect physical input, no stuck input. Stress resize/search/open-close overlays during playback, repeated play/pause and focus loss, large catalog, idle. Native checks only in controlled test environment.

## 8. Implementation work packages (no code changes yet)

| WP | Priority | Deliverable | Dependencies |
|---|---|---|---|
| G2-00 | P0 | Capture revision-matched baseline; run current suite and native baseline; preserve existing contracts | none |
| G2-01 | P1 | Dual appearance tokens, typography and surface hierarchy; parity and palette isolation | G2-00 + palette sign-off |
| G2-02 | P1 | Workbench chrome, navigation, tracks and inspector hierarchy | G2-01 |
| G2-03 | P0/P1 | Playback identity/status, transport, profile and admission/recovery | G2-00 + CTA decision; G2-01 for visuals |
| G2-04 | P1 | Motion aliases + interruption/reduced-motion verification | G2-01, affected component contracts |
| G2-05 | P2 | Settings, appearance, diagnostics, update/empty/error polish | G2-01–04 as relevant |
| G2-06 | P0 | Full regression and native qualification, source-matched screenshots | all above |

Separate PRs by work package. No unrelated dependency upgrades, business logic changes or test weakening. Each PR states pre/post screenshots, affected states and evidence. P0 UX meaning/CTA may be validated before cosmetic work. Phase 0 is a gate, not a requirement to redesign before evidence exists.

## 9. Test matrix and traceability

| Coverage | Existing tests to extend / rerun | New verification |
|---|---|---|
| Tokens and accessibility | `theme-accessibility.spec.ts`, `status-badge-theme.spec.ts`, `graphiteThemeTokens.test.ts` | candidate rendered contrast all states in Dark and Light; other dark palettes computed colors unchanged; forced colors; first paint and effective mode |
| Workbench/compact | `workbench-layout.spec.ts`, `workbench-resize.spec.ts`, `workbench-state-regression.spec.ts`, `library-layout.spec.ts` | default/min/large, pane interaction, DPI native |
| Search/library | `library.spec.ts`, `locate-selected.spec.ts`, `track-title-tooltip.spec.ts`, `titlebar.spec.ts` | long titles, hidden actions, keyboard, native chrome |
| Transport | `player-dock-density.spec.ts`, `playback.spec.ts`, `PlayerTransport.test.tsx`, `PlayerTrackInfo.test.tsx` | selected≠playing, mode copy, geometry stable, rapid command cases |
| Admission | `playback-confirmation-layout.spec.ts`, `playback-recovery.spec.ts`, `sheet-compatibility.spec.ts`, `PlaybackAdmission.test.tsx` | decision variants and cancel/test/proceed, native no-input verification |
| Inspector/settings | `settings-diagnostics.spec.ts`, `design-system-finishing.spec.ts`, `system-appearance.spec.ts` | correct session context, popup interruption, all modes |
| Update/startup | `update-modal-geometry.spec.ts`, `startup-fallback.spec.ts`, bundle tests | failed update, scroll, native reveal, no extra splash delay |
| Visual evidence | `visual-evidence.spec.ts`, `bun run ui:refresh` | all canonical PNGs + relevant VQC scenes with matching tree SHA; paired Dark/Light captures for workbench, compact, selection/playing/focus, profile, inspector, admission, settings, update, error and empty/loading. Preserve canonical filenames and manifest contract; extend targeted evidence deliberately. |

Test exact behavior from repository scripts, not inferred command names. Run existing lint/type/unit/e2e checks per package.json. Before merging, document test failures, intentional screenshot changes and evidence not available. Capture browser and native separately. If a new visual decision conflicts with current approved tests, obtain sign-off for the changed contract and update assertions transparently; don't simply relax test thresholds.

## 10. Sign-off decisions and definition of done

### Visual acceptance addendum (v1.1)

Compare matched scenes, identical viewport, fixture, selection and playback state, Dark and Light side by side. Main area must be the content anchor rather than a raised card; normal side panes and dock share chrome tone, all separators quiet but visible, no periwinkle tint across the whole shell, no extra album-art tile, no nested card stack or added toolbars. Both modes must use the same component geometry, accessible state semantics and copy; only color, shadow/edge contrast and necessary brand asset variant change. Check active/idle titlebar, search focus, selected+playing row, idle/hover/pressed controls, status warnings, scrollbar and popup overlays. At 800×560 compare number of visible rows and transport accessibility to baseline; do not sacrifice usable space just for polish. In every paired scene, verify keyboard focus and no horizontal overflow. Follow Windows OS transitions while settings/profile are open and playback active: preserve state and scroll, no new input, no app-wide fade, no flash. Test saved non-Graphite palette -> Light -> Dark restores original palette. No claim of native pass until actual native qualification.


Product decisions proposed for approval: (A) paired neutral Dark/Light palettes, existing periwinkle/indigo action tokens, subtle pane seams and restrained radius, (B) 46px row retained with primary titles, (C) fixed full-width dock rather than floating dock in v1, (D) test-first CTA when risk/available dry-run choice, after domain review, (E) no global theme crossfade or additional animation library, (F) retain current scrollbar reveal policy and defer density preferences. Illustrative concepts from earlier audit are not production layout requirements. If design review rejects candidate palette, retain existing semantic colors and improve hierarchy first. The screenshot is the style target; no actual implementation or native evidence yet.

Done when revision-matched screenshots show approved design, all existing and new regressions pass, native tests confirm no timing/safety regression, accessibility checks pass for tested cases, forced colors and reduced motion behave correctly, and product decisions are signed off. No unverified claim of WCAG certification or native performance.

## 11. References

- Microsoft, Modernizing the VS Code UI (2026-10-15): https://code.visualstudio.com/blogs/2026/10/15/modern-ui
- VS Code 2026 Dark: https://github.com/microsoft/vscode/blob/main/extensions/theme-defaults/themes/2026-dark.json
- VS Code 2026 Light: https://github.com/microsoft/vscode/blob/main/extensions/theme-defaults/themes/2026-light.json
- VS Code Theme Color: https://code.visualstudio.com/api/references/theme-color
- VS Code Views UX guidelines: https://code.visualstudio.com/api/ux-guidelines/views
- Fluent 2 alias color tokens: https://fluent2.microsoft.design/color-tokens/


- Microsoft Windows design guidelines: https://learn.microsoft.com/en-us/windows/apps/design/guidelines-overview
- Windows title bar: https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design
- Windows typography: https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography
- Fluent 2 design tokens: https://fluent2.microsoft.design/design-tokens/
- Apple HIG Dark Mode: https://developer.apple.com/design/human-interface-guidelines/dark-mode
- Apple HIG Motion: https://developer.apple.com/design/human-interface-guidelines/motion
- W3C WCAG 2.2: https://www.w3.org/TR/WCAG22/
- W3C focus not obscured: https://www.w3.org/WAI/WCAG22/Understanding/focus-not-obscured-minimum
- W3C non-text contrast: https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html
- web.dev animation performance: https://web.dev/articles/animations-guide

External references support design principles; all concrete project token values, geometry candidates, motion duration targets, work packages and acceptance thresholds are proposed specifically for Sky Auto Player.

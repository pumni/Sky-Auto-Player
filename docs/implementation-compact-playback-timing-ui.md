# Compact playback timing UI handoff

Status: **implementation work order for Codex**.

Base: `main` after PR #233.

This PR is deliberately UI-only. It must improve the quick playback profile popover and the Playback section of Settings without changing timing policy, persistence, native/runtime behavior, calibration semantics, or playback admission.

## User problem

The timing controls added in PR #233 are functionally correct but visually too large and verbose.

Two current problems are visible in the shipped layout:

1. The quick playback profile popover at the lower-right grows much wider than the controls need. The current combination of `width: max-content`, a full-width **Use recommended (...)** button, and long explanatory copy makes the popover dominate the workbench.
2. The Playback page in Settings gives the Timing Margin recommendation too much visual weight and spreads the timing information over too much space.

The target is a compact, high-information layout that keeps common controls immediately readable and moves explanation into secondary text.

## Non-goals / invariants

Do **not** change any of the following:

- Timing Margin default: **500 us**.
- Late Down tolerance default: **2000 us**.
- Timing Margin range/step.
- Late Down tolerance range/step.
- Base Hold choices.
- FPS or tempo choices.
- Authored Hold / Release Gap equations.
- Calibration or recommendation calculation.
- Persisted settings schema.
- Native bridge DTOs.
- Session freezing behavior.
- Scheduler/compiler behavior.
- SendInput, cutoff, admission, recovery, packet, chord, scan-code, retry, or telemetry semantics.

This PR is a presentation and interaction-density change only.

## Primary implementation areas

Expected files:

- `desktop/src/components/player/PlayerTools.tsx`
- `desktop/src/components/settings/TimingMarginControl.tsx`
- `desktop/src/components/settings/SettingsPanel.tsx`
- `desktop/src/styles/player.css`
- `desktop/src/styles/overlays.css`
- related component tests
- `desktop/tests/e2e/library.spec.ts` if geometry/responsive assertions need updating

Avoid native Rust changes unless a test proves a UI contract cannot otherwise be preserved. Any Rust change requires explicit reviewer justification.

## 1. Quick playback profile popover

The quick profile is a fast-access surface, not a full explanation surface.

### Required layout

Make the popover compact and bounded.

Target behavior:

- It must no longer size itself from long recommendation/help text.
- Prefer an explicit compact width in the **260-320 px** range.
- It may shrink responsively on narrow windows.
- It must never expand across most of the track list as in the current implementation.
- Long copy must wrap inside the bounded width rather than determine the width.

Replace the current vertical/full-width recommendation treatment.

Current pattern to remove from the quick popover:

```
Timing Margin                    [-] 800 us [+]
[        Use recommended (2300 us)               ]
Applies to Hold and Release Gap...
Recommended sender margin: 2300 us · Source...
```

Target pattern:

```
Base Hold                 [1f]
Tempo                     [1x]
FPS                       [60]

Timing Margin · rec. 2300 us     [-] 800 us [+]

Target hold               17.167 ms
Release gap               17.167 ms
Late Down tolerance       2000 us

[ Test playback (no input) ]
```

Exact typography may be refined, but the information hierarchy is required.

### Recommendation behavior in quick profile

Remove the **Use recommended** button from the quick profile.

The quick profile must show recommendation as non-interactive secondary text immediately adjacent to the Timing Margin label, for example:

- `Timing Margin · rec. 2300 us`
- or `Timing Margin   Recommended 2300 us`

Requirements:

- Recommendation remains visible.
- It must not look clickable.
- If recommendation is outside the valid Timing Margin range, show that state succinctly, e.g. `rec. 5300 us · out of range`.
- Do not silently clamp the recommendation.
- The quick profile must not offer an action that writes an out-of-range recommendation.

The quick profile does **not** need the recommendation source or the long “sender evidence, not proof...” paragraph.

### Quick timing summary

Reduce the summary to the values most useful while choosing a profile:

- Target hold.
- Release gap.
- Late Down tolerance.

The following are redundant in the quick surface and should be omitted there:

- `1 frame`
- `Base hold`

They may remain in the full Settings view.

### Control density

Base Hold, Tempo, and FPS must remain immediately editable.

Use compact rows or another dense arrangement that preserves readable labels and keyboard access. Do not create a large card for each field.

The Timing Margin stepper remains authoritative and keeps the existing async write/reconciliation behavior.

The full-width **Test playback (no input)** action remains at the bottom.

## 2. Playback section in Settings

The Settings modal is the full-detail surface, but it should still be visually economical.

### Required hierarchy

Keep:

- Playback defaults heading.
- Base Hold / Tempo / FPS controls.
- Timing Margin control.
- Full timing summary.
- “next prepared session / frozen active session” semantics.

Improve the layout so Timing Margin reads as one settings row instead of a large action block.

Suggested structure:

```
Playback defaults
[ Base Hold ] [ Tempo ] [ FPS ]

Timing
Timing Margin     rec. 2300 us                 [-] 800 us [+]
                  Default fallback · sender-side recommendation
                  Applies to Hold and Release Gap

1 frame          16.667 ms      Base hold       16.667 ms
Target hold      17.467 ms      Release gap     17.467 ms
Late Down tolerance             2000 us

Playback changes apply to the next prepared session...
```

The exact visual grouping can differ, but the following acceptance rules are mandatory:

- No full-width **Use recommended** button.
- Recommendation text must be visually secondary to the current setting.
- Recommendation source may remain in Settings, but it must be short and subordinate.
- The long sender-evidence disclaimer must not dominate the section.
- Current value and +/- controls must be visually aligned as one row.
- Timing summary values must be compact, aligned, and scan-friendly.
- Avoid large unused horizontal gaps.
- Do not make the modal wider to solve the layout.

### Recommendation application

For this UI cleanup, recommendation is informational only.

Remove direct **Use recommended** application from `TimingMarginControl` rather than keeping a hidden/context-dependent action.

The user can set the recommended number with the existing +/- stepper.

Do not add a new icon-only apply button, context menu, or hover-only action.

This simplification is intentional.

### Settings dialog dimensions

Do not increase the existing Settings dialog footprint.

It is acceptable to reduce its width slightly if the result remains comfortable for all categories. The Playback section must work without horizontal scrolling.

At approximately 1280x720:

- Settings must remain fully usable.
- Footer and close control must remain reachable.
- Playback content may vertically scroll if needed.
- No field or stepper may be clipped.

## 3. Shared component design

`TimingMarginControl` is currently shared by Settings and PlayerTools. Refactor it so the same state/reconciliation logic can support two presentation densities without duplicating async setting logic.

Preferred approaches include:

- a `density="compact" | "full"` / `variant` prop;
- small presentational subcomponents over one shared request/reconciliation hook.

Do not copy/paste the pending-write logic into PlayerTools.

The recommendation must remain readable to assistive technology in both variants.

## 4. Accessibility

Preserve or improve:

- `aria-label="Timing Margin"`.
- Decrease/Increase button accessible names.
- `output aria-live="polite"` for current value.
- Keyboard operation for selects and stepper buttons.
- Visible focus states.
- Dialog/Popover focus behavior.

Recommendation text must not be announced as an actionable control after the button is removed.

## 5. Responsive / geometry acceptance

Add or update automated coverage where practical.

Acceptance targets:

### Quick profile

At standard desktop widths:

- bounded compact width, approximately 260-320 px;
- no horizontal page overflow;
- no overlap beyond the viewport edge;
- long recommendation/source strings cannot expand the popover width.

At narrower supported window sizes:

- popover remains within viewport;
- controls remain operable;
- text may wrap;
- no horizontal scrollbar inside the popover unless absolutely unavoidable.

### Settings

- no horizontal overflow in Playback;
- three primary selects may collapse responsively if existing breakpoints require it;
- timing stepper stays aligned and usable;
- summary remains readable.

## 6. Tests

Update tests to reflect the interaction simplification.

Required component coverage:

- current Timing Margin renders correctly;
- +/- update behavior and pending-write reconciliation remain unchanged;
- recommendation renders as text;
- **Use recommended** button is absent;
- out-of-range recommendation is visible as informational state and cannot write an invalid value;
- compact and full variants render the intended information;
- Settings still renders source/qualification information in a subordinate form;
- quick profile omits long source/evidence copy;
- quick profile retains Test playback behavior.

Run at minimum:

```powershell
cd desktop
bun test
bun run build
bunx playwright test
```

Then run the repository's canonical desktop/CI validation path used by GitHub Actions.

## 7. Visual acceptance checklist

Before declaring complete, manually inspect the actual Tauri app at the same approximate layouts as the reported screenshots.

Quick profile acceptance:

- [ ] popover is visibly much narrower than before;
- [ ] no **Use recommended** button;
- [ ] recommendation is beside Timing Margin as small secondary text;
- [ ] no long recommendation/source paragraph;
- [ ] Target hold / Release gap / Late Down tolerance remain visible;
- [ ] Test playback stays obvious;
- [ ] no clipping at right/bottom edge.

Settings acceptance:

- [ ] Playback page has a clear primary/secondary hierarchy;
- [ ] Timing Margin occupies one compact row;
- [ ] recommendation does not look like the primary action;
- [ ] summary is aligned and compact;
- [ ] no unnecessary width increase;
- [ ] no behavior regression when changing Base Hold, Tempo, FPS, or Margin.

Capture before/after screenshots in the final Codex report if the local environment allows it.

## 8. Scope discipline

Do not use this PR to redesign the entire app.

Do not:

- redesign navigation;
- restyle unrelated dialogs;
- change themes/tokens globally unless a narrowly scoped token is necessary;
- move Late Down tolerance between categories as part of this work;
- change playback behavior;
- change recommendation math.

If a broader design issue is discovered, report it separately rather than expanding this PR.

## Definition of Done

The implementation is ready for independent review when:

1. quick profile is compact and bounded;
2. recommendation is informational text, not a button;
3. quick profile no longer contains verbose recommendation copy;
4. Settings Playback timing hierarchy is visibly cleaner;
5. all timing values and persistence semantics are unchanged;
6. component/unit/E2E tests pass;
7. exact-head CI is green;
8. final report contains changed files, validation commands, exact head SHA, CI run, and screenshots or a concise visual verification statement.

Do not merge this PR. Leave it draft for independent acceptance.

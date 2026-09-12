# User-owned symmetric Timing Margin implementation handoff

## Status

Implementation handoff for Codex.

- Repository: `pumni/Sky-Auto-Player`
- Predecessor scheduler PR: #232, merged into `main` as `0a428cd1e2c5da93890e81c46feb6b5c7f514fca`
- Work branch: `docs/user-owned-timing-margin-handoff-2026-09`
- Handoff base: `0a428cd1e2c5da93890e81c46feb6b5c7f514fca` (`main` after PR #232 merge)
- Target platform: Windows 11 x86_64
- Rust workspace: Edition 2024, Rust 1.98 / pinned toolchain 1.98.1
- Physical input boundary: Windows `SendInput`
- PR must remain draft until an independent review explicitly clears it.

Before editing, read `AGENTS.md`, `SECURITY.md`, `docs/hold-frame-model.md`,
`docs/timing-principles.md`, `docs/rt-dispatch-architecture.md`, the existing
scheduler handoff, and the direct tests for every changed boundary.

This handoff starts from the scheduler reliability implementation already merged by PR #232. It supersedes only the hidden hold/release headroom semantics; all accepted scheduler reliability, Diagnostics, UIPI, heartbeat and physical-send invariants remain in force.

---

## Product decision — locked

The following decisions are already made. Do not reopen them unless an
implementation contradiction is found and documented.

1. Game timing is modeled in frames. Empirical game testing has established
   that a key hold shorter than one game frame can fail to register.
2. The existing base hold choices remain exactly:
   - `1.0 frame`
   - `1.25 frames`
   - `1.5 frames`
3. Add one user-owned `Timing Margin`, expressed in integer microseconds.
4. The Timing Margin applies symmetrically:
   - Down -> Up hold
   - Up -> next same-key Down release gap
5. Initial default Timing Margin is **800 us**.
6. UI adjustment step is **100 us**.
7. Initial supported range is **0..=3000 us**.
8. `0 us` is valid. Do not silently clamp it upward.
9. Production Down late cutoff remains **500 us**.
10. Down late cutoff is not part of Timing Margin and must not change when the
    user changes Timing Margin.
11. Calibration may recommend a Timing Margin but must never silently change
    the user's selected Timing Margin.
12. No hidden hold/release addition is allowed after materialization. The
    authored formula shown to the user must be the formula used by planning.
13. Do not implement automatic hold compression.
14. Do not implement adaptive runtime changes to Timing Margin.
15. Do not expose Down late cutoff as an editable normal-user setting in this
    work.
16. Preserve `SendInput` as the only gameplay input boundary.

---

## Target timing model

Definitions:

```text
frame_us = ceil(1_000_000 / fps)
frame_base_hold_us = ceil(hold_frames * frame_us)

timing_margin_us = exact persisted user value

effective_min_hold_us =
    frame_base_hold_us + timing_margin_us

min_release_gap_us =
    frame_us + timing_margin_us
```

There must be no additional:

```text
+ down_late_grace_us
+ transport_margin_us
```

inside either authored timing equation.

The Down deadline remains a separate dispatch rule:

```text
latest_allowed_down_qpc =
    physical_target_qpc + down_late_grace_us

production down_late_grace_us = 500 us
```

This separation is an invariant.

### Example: 60 FPS, default profile

```text
frame_us            = 16,667 us
hold_frames         = 1.0
timing_margin_us    =    800 us

target hold         = 17,467 us
release gap         = 17,467 us
Down late cutoff    =    500 us
```

This intentionally preserves the current fallback/default authored hold and
release timing (`500 + 300 = 800 us`) while changing ownership and semantics:
the 800 us is now explicit user configuration rather than hidden engine-added
headroom.

### Example: 60 FPS, aggressive user profile

```text
hold_frames         = 1.0
timing_margin_us    = 0

target hold         = 16,667 us
release gap         = 16,667 us
Down late cutoff    =    500 us
```

This is legal. If a Down is actually sent late, sender-observed physical hold
may fall below one frame. Diagnostics must make that observable; the engine must
not secretly lengthen the authored hold.

---

## Semantics: user setting vs recommendation vs dispatch policy

Keep these three concepts distinct everywhere in names, DTOs and tests.

### 1. User Timing Margin

Authoritative for schedule construction:

```text
timing_margin_us
```

Properties:

- persisted;
- exact integer microseconds;
- default 800;
- range 0..=3000;
- multiple of 100;
- session-frozen once prepared/started;
- included in schedule identity/fingerprint/cache inputs;
- changed only by explicit settings action.

### 2. Recommended Timing Margin

Advisory only:

```text
recommended_timing_margin_us
```

Derived from calibration evidence and the fixed late window. It must not feed
directly into planning.

A suitable recommendation rule is:

```text
raw_recommended =
    DEFAULT_DOWN_LATE_GRACE_US
    + calibrated_or_fallback_transport_reserve_us

recommended =
    ceil_to_100us(raw_recommended)
```

Current fallback transport reserve is 300 us, therefore fallback recommendation
is exactly 800 us.

The existing calibration validity/qualification status must remain visible.
An out-of-envelope/unqualified calibration may fall back to the baseline
recommendation, but must not be presented as qualified device evidence.

The current calibrated transport candidate is at most 2000 us when qualified,
so `500 + 2000 = 2500 us`, within the new 3000 us UI range.

### 3. Down late cutoff

Dispatch policy only:

```text
down_late_grace_us = 500
```

It decides whether a Down that missed its target is still allowed to reach
`SendInput`. It is read-only user-visible diagnostic information, not a
component of authored hold/release timing.

Changing the user margin must never change this cutoff. Changing the test-only
Down grace seam must never recompute hold/release targets.

---

## Data model and persistence

### sky_app_core settings

Primary file:

- `rust/crates/sky_app_core/src/settings.rs`

Add:

```rust
pub const DEFAULT_TIMING_MARGIN_US: u64 = 800;
pub const MIN_TIMING_MARGIN_US: u64 = 0;
pub const MAX_TIMING_MARGIN_US: u64 = 3_000;
pub const TIMING_MARGIN_STEP_US: u64 = 100;
```

Extend `PlaybackDefaults`:

```rust
pub struct PlaybackDefaults {
    pub hold_frames: f64,
    pub timing_margin_us: u64,
    pub tempo_scale: f64,
    pub fps: u16,
}
```

Extend `PlaybackDefaultsPatch` with `timing_margin_us: Option<u64>`.

Validation contract:

```text
0 <= timing_margin_us <= 3000
timing_margin_us % 100 == 0
```

Reject invalid API patches. Do not silently round explicit patch values.

### settings schema migration

Bump the durable settings schema version according to repository migration
conventions.

Old configs that do not contain the new field must migrate to:

```text
timing_margin_us = 800
```

This is required to preserve the previously qualified fallback/default authored
timing.

Do not infer the persisted user value from calibration cache contents. Device
calibration remains advisory evidence, not user configuration.

Update:

- native settings adapter load/save/migration;
- fixtures and golden settings documents;
- settings normalization;
- patch tests;
- any compatibility tests that assume the old schema.

---

## Materialized timing policy

Primary file:

- `rust/crates/sky_app_core/src/timing.rs`

Target applied policy should carry the actual authored timing inputs explicitly:

```rust
pub struct MaterializedTimingPolicy {
    pub fps: u16,
    pub frame_us: u64,
    pub hold_frames: f64,
    pub frame_base_hold_us: u64,
    pub timing_margin_us: u64,

    // separate dispatch policy:
    pub down_late_grace_us: u64,

    pub min_hold_us: u64,
    pub min_release_gap_us: u64,
    pub focus_restore_grace_us: u64,
}
```

The exact field layout may differ if compatibility requires additional fields,
but the applied timing equations must be:

```rust
min_hold_us =
    frame_base_hold_us
        .checked_add(timing_margin_us)
        ?;

min_release_gap_us =
    frame_us
        .checked_add(timing_margin_us)
        ?;
```

Prefer checked arithmetic at validation/materialization boundaries. Do not rely
on saturation to turn malformed settings into a valid schedule.

### Remove calibration from applied timing

The current `from_calibration(...)` design couples transport evidence to
authored timing. Replace/refactor it so normal production materialization takes
the explicit user `timing_margin_us`.

Calibration data may still be loaded by the desktop/native adapter to produce
the recommendation presented to the user.

Do not delete useful raw calibration evidence, qualification rules or cache
validation merely because it no longer directly changes scheduling.

### Critical regression test

The existing test seam that materializes an explicit Down grace must be changed
so that:

```text
Down grace 500 / 750 / 1000
DOES change native Down cutoff
DOES NOT change min_hold_us
DOES NOT change min_release_gap_us
```

This is one of the most important tests in the change.

---

## Song planning and feasibility

Primary file:

- `rust/crates/sky_app_core/src/song.rs`

Retain the current three hold-frame choices.

Planning contract:

```text
target_hold_us = policy.min_hold_us
min_hold_us    = policy.min_hold_us
release_gap_us = policy.min_release_gap_us
```

There is no automatic compression below the user's configured timing.

For same-key notes:

```text
authored Up - authored Down >= base_hold + timing_margin
next same-key Down - previous Up >= 1 frame + timing_margin
```

If the interval is infeasible, preserve fail-closed/preparation-time reporting.
Do not silently reduce `hold_frames` or `timing_margin_us`.

Update risk/recommendation wording so it does not describe the old hidden
Down-grace-plus-transport aggregate as the user's hold margin.

The margin is symmetric. A +100 us user change increases minimum same-key cycle
by 200 us when base hold is unchanged.

---

## Native boundary and runtime

The real-time worker architecture must remain intact.

Preserve:

- absolute authored targets;
- high-resolution waitable timer;
- bounded QPC spin;
- final command/control gate;
- exact target HWND generation validation;
- final foreground/focus validation;
- supervisor lease;
- true pre-call QPC cutoff;
- single `SendInput` call per prepared packet;
- no allocation/log formatting on RT path;
- fail-closed partial/zero-progress behavior;
- existing key ownership/recovery semantics.

### Down cutoff

Production remains:

```text
500 us
```

No user Timing Margin is added to the cutoff.

### Up behavior

Do not create a new runtime delay to enforce hold/release durations. They remain
authored schedule constraints validated before execution.

### Session freeze

The active/prepared session must contain the exact materialized user margin.
Changing settings must not mutate an already-running session.

If a playback-affecting setting changes while a prepared plan exists, invalidate
that prepared plan using the existing store/runtime invalidation pattern.

---

## Calibration refactor

Relevant areas include:

- `rust/crates/sky_dispatch_win32/src/calibration.rs`
- native adapter calibration cache handling;
- `desktop/src-tauri/src/native_runtime.rs`
- calibration tests/fixtures.

Keep sender evidence:

```text
T_D / P_D / C_D
T_U / P_U / C_U
scheduler lateness
SendInput call duration
sender hold shrink identity
qualification buckets
hot/cold evidence
```

Change only how the result is consumed.

Old direction:

```text
calibration
  -> transport_margin_us
  -> hidden addition to materialized hold/release
```

Target direction:

```text
calibration
  -> transport reserve evidence
  -> recommended_timing_margin_us
  -> UI recommendation
  -> user may explicitly apply it
```

No calibration completion event may silently patch `timing_margin_us`.

The UI action `Use recommended` may explicitly patch the setting.

Recommendation should round upward to the nearest 100 us so an advisory value
never rounds below its evidence budget.

---

## Tauri / DTO contract

Primary areas:

- `desktop/src-tauri/src/commands.rs`
- `desktop/src-tauri/src/native_runtime.rs`
- `desktop/src-tauri/src/ui_events.rs`
- generated bindings

Extend the playback/default/config DTOs so `timing_margin_us` travels end to
end:

```text
settings
 -> bootstrap
 -> React store
 -> PlaybackConfig
 -> prepare/analyze
 -> materialized timing policy
 -> session
```

Do not hand-edit files under `desktop/src/bridge/generated/`.

Run:

```powershell
cargo xtask bindings generate
```

Update protocol/schema versions if required by the repository's existing
compatibility rules.

### Option metadata

Expose enough metadata so React does not hard-code policy independently:

```text
timing_margin_min_us  = 0
timing_margin_max_us  = 3000
timing_margin_step_us = 100
```

Exact DTO shape may use a small range object if that matches current project
style.

---

## UI/UX

Primary areas:

- `desktop/src/components/player/PlayerTools.tsx`
- `desktop/src/components/settings/SettingsPanel.tsx`
- state store and tests
- workbench styles
- mock bridge

Playback Profile should expose:

```text
FPS
[ 60 ]

Base Hold
[ 1.0 frame ]

Timing Margin
[ - ]  +800 us  [ + ]

Tempo
[ 1.00x ]
```

The exact visual style should match the existing UI.

### Required behavior

- minus/plus changes by exactly 100 us;
- clamp buttons at 0 and 3000 by disabling the unavailable direction;
- do not send invalid intermediate values;
- show the current value in a human-readable form;
- `800 us` may be shown as `0.8 ms` in the primary UI, but exact microseconds
  should remain available in details/Diagnostics;
- provide `Use recommended` when current != recommendation;
- applying recommendation is an explicit user action;
- show that playback changes apply to the next prepared session;
- do not mutate active session timing.

Suggested profile summary:

```text
1f + 0.8ms · 1.00x · 60 FPS
```

### Explain the result, not hidden implementation

Show a compact derived timing block where practical:

```text
1 frame          16.667 ms
Base hold        16.667 ms
Timing margin    +0.800 ms
Target hold      17.467 ms
Release gap      17.467 ms
```

Read-only detail:

```text
Down late cutoff 500 us
Recommended      800 us
Recommendation source/status: fallback or qualified calibration
```

Do not label the user's margin as `Down grace` or `transport margin`.

---

## Diagnostics

Diagnostics must distinguish **configuration** from **observation**.

### Configuration/session facts

For an active session, publish actual frozen values, not current editable
defaults:

```text
fps
frame_us
hold_frames
frame_base_hold_us
timing_margin_us
min_hold_us / target hold
min_release_gap_us
down_late_grace_us
recommended_timing_margin_us
recommendation qualification/source
```

### Observed sender evidence

Keep the production evidence already added in PR #232:

```text
fine pre-call lateness buckets
missed Down taxonomy
hard-late / final-cutoff counters
partial / zero-progress SendInput evidence
hold/release/retrigger forensics
stuck/ownership anomalies
last error where already supported
```

The UI should make the following situation understandable:

```text
Configured margin:          0 us
Down late cutoff:         500 us
Observed max Down late:   420 us
Sender hold below frame:    1
```

This is evidence for the user to increase margin; the engine must not
automatically do so.

Diagnostics remains observational and must not enable
`StrictTimingDiagnostic`.

---

## Fingerprints, cache identity and reproducibility

`timing_margin_us` changes authored Up timestamps and same-key feasibility.
Therefore it must participate in every identity that currently incorporates the
timing policy, including as applicable:

- prepared playback fingerprint;
- analysis cache key;
- schedule cache identity;
- decision/acceptance identity;
- diagnostic/session metadata;
- golden fixtures.

Calibration recommendation must **not** participate in schedule identity unless
it was explicitly applied to `timing_margin_us`.

Changing calibration evidence alone must not invalidate a prepared schedule
whose user margin is unchanged.

---

## Documentation update

Update at least:

- `docs/hold-frame-model.md`
- `docs/timing-principles.md`
- `docs/rt-dispatch-architecture.md`

The documentation must stop claiming:

```text
effective hold = frame base + Down grace + transport margin
release gap    = frame + Down grace + transport margin
```

and instead document:

```text
effective hold = frame base + user Timing Margin
release gap    = one frame + user Timing Margin

Down late cutoff = independent fixed dispatch policy, currently 500 us
calibration = recommendation/evidence only
```

Retain historical perf-baseline documents unchanged except for a new follow-up
report. Historical evidence must remain historically accurate.

---

## Required tests

### Core settings

- default margin is 800;
- 0 accepted;
- 3000 accepted;
- 100-step values accepted;
- 1 / 799 / 801 / 3001 rejected as appropriate;
- old settings migrate to 800;
- round-trip persistence preserves exact value;
- patch atomicity preserved.

### Timing materialization

At 60 FPS:

```text
1.0f + 0    -> hold 16667, gap 16667
1.0f + 500  -> hold 17167, gap 17167
1.0f + 800  -> hold 17467, gap 17467
1.25f + 800 -> hold 21634, gap 17467
1.5f + 800  -> hold 25801, gap 17467
```

Also cover 30/90/120/144/165/240 FPS.

### Independence invariant

For the same FPS/hold/margin, test Down grace 500/750/1000:

```text
min_hold_us identical
min_release_gap_us identical
only native late cutoff changes
```

### Calibration

- qualified evidence updates recommendation only;
- fallback recommendation is 800;
- recommendation rounds upward to 100-us step;
- applying recommendation requires explicit settings patch;
- calibration does not mutate active/prepared timing by itself.

### Planning

- symmetric margin is applied to both hold and release;
- +100 margin increases minimum 1f same-key cycle by 200 us;
- infeasible repeat remains reported/rejected according to current planner
  contract;
- no automatic compression is introduced.

### Fingerprint/cache

- changing 800 -> 900 changes timing identity;
- changing recommendation only does not change timing identity;
- changing current settings does not mutate an already frozen session.

### UI

- +/- exact 100-us steps;
- lower/upper buttons disable correctly;
- result timing updates correctly with FPS/base/margin;
- recommendation display and `Use recommended`;
- active session remains frozen;
- profile summary includes margin;
- generated DTOs match Rust.

### Existing scheduler invariants

All existing PR #232 tests must remain green, especially:

- exact cutoff boundary;
- one QPC tick beyond cutoff;
- target HWND race;
- focus loss;
- lease expiry;
- pause release sweep;
- stop/skip cleanup;
- partial/zero-progress SendInput;
- chord one-call behavior;
- allocation-free production RT path.

---

## Physical Windows acceptance

Because applied production timing plumbing changes, do not rely solely on old
physical evidence even though the default authored duration is preserved.

### Mandatory default regression

Run the existing real `SendInput` acceptance harness on Windows with:

```text
FPS            60
Base Hold      1.0f
Timing Margin  800 us
Down cutoff    500 us
Production profile/backend
StrictTimingDiagnostic disabled
```

Run the full existing 13-scenario matrix and retain raw reports/event logs.

Expected:

- no partial/zero-progress;
- no stuck keys;
- no ownership/retrigger anomalies;
- no unexpected hard-late/cutoff/backlog miss;
- lifecycle/focus/target scenarios preserve current behavior.

### Focused margin sweep

At minimum run sender-side targeted hold/retrigger scenarios with:

```text
0 us
500 us
800 us
1000 us
3000 us
```

Verify authored and observed intervals reflect the configured value exactly;
there must be no hidden +500/+300 addition.

This sink evidence does not prove Sky game observation. Document that
limitation. The one-frame game-registration premise comes from separate manual
game testing and is not to be reinterpreted as a WinForms-sink claim.

If a physical Windows environment is unavailable, code/CI may be completed but
release acceptance remains blocked until this evidence exists.

---

## Non-goals

Do not add any of the following in this task:

- editable Down late cutoff in normal UI;
- separate Hold Margin and Release Margin;
- negative margin;
- arbitrary microsecond text entry;
- automatic runtime margin adaptation;
- silent calibration application;
- automatic hold compression;
- per-note dynamic hold;
- game-memory observation;
- hooks/injection;
- a second input backend;
- changes to production 500-us Down cutoff;
- changes to MMCSS/QoS/waiter policy unless required by a demonstrated bug.

---

## Suggested implementation sequence

1. Add settings field, validation and schema migration.
2. Refactor `MaterializedTimingPolicy` to consume explicit user margin.
3. Decouple Down grace from authored hold/release equations.
4. Refactor calibration output into recommendation-only semantics.
5. Thread margin through playback DTO/config/store/fingerprint/cache.
6. Add UI +/- control and recommendation action.
7. Extend Diagnostics with exact configured/session values.
8. Update docs and fixtures.
9. Run bindings generation.
10. Run focused tests.
11. Run `cargo xtask check all`.
12. Run fresh GitHub Actions on the exact implementation head.
13. Run physical Windows acceptance and check in evidence.

Prefer small reviewable commits. A reasonable split is:

```text
feat(settings): add user-owned timing margin
refactor(timing): decouple authored margin from down cutoff
refactor(calibration): make timing margin advisory
feat(desktop): expose timing margin controls and diagnostics
test(timing): qualify symmetric user margin policy
docs(timing): document user-owned timing margin
```

Do not force this split if atomic dependencies require a different order.

---

## Acceptance checklist

Implementation is acceptable only when all are true:

- [ ] Base choices remain 1.0 / 1.25 / 1.5 frames.
- [ ] Default persisted Timing Margin is 800 us.
- [ ] Range is 0..=3000 us; normal UI step is 100 us.
- [ ] Hold = base frame hold + exact user margin.
- [ ] Release gap = one frame + exact same user margin.
- [ ] No hidden Down-grace addition to hold/release remains.
- [ ] No hidden transport/calibration addition to hold/release remains.
- [ ] Production Down cutoff remains 500 us.
- [ ] Down cutoff and Timing Margin are independent.
- [ ] Calibration changes recommendation only.
- [ ] User explicitly chooses whether to apply recommendation.
- [ ] Prepared/running session timing is immutable.
- [ ] Planner does not silently compress user timing.
- [ ] Margin participates in schedule/fingerprint/cache identity.
- [ ] Recommendation alone does not participate in schedule identity.
- [ ] Diagnostics shows actual frozen margin/hold/gap/cutoff.
- [ ] Generated bindings are regenerated, not hand-edited.
- [ ] RT no-allocation and one-`SendInput` invariants remain.
- [ ] Full local validation passes.
- [ ] Fresh CI passes on exact implementation head.
- [ ] Default 800-us real-Windows regression evidence passes.
- [ ] Focused margin sweep proves no hidden timing addition.
- [ ] PR remains draft until explicit independent acceptance.

---

## Codex completion report contract

When implementation is complete, report back on the implementation PR created for this handoff with:

1. exact implementation head SHA;
2. exact commit list added for this handoff;
3. exact changed-file list;
4. concise description of final timing equations;
5. settings schema/migration behavior;
6. calibration recommendation behavior;
7. UI behavior;
8. diagnostics behavior;
9. local commands and results;
10. GitHub Actions run ID and exact-head verification;
11. physical acceptance report paths and raw evidence paths;
12. any deviation from this handoff;
13. any unresolved blocker.

Keep the implementation PR created for this handoff in draft until explicit independent acceptance. Do not merge it as part of Codex implementation.

# Real-game input reliability investigation and user-owned Late Down tolerance

## Status

Implementation / investigation handoff for Codex on draft PR #233.

- Repository: `pumni/Sky-Auto-Player`
- PR: #233, currently OPEN + DRAFT
- Branch: `docs/user-owned-timing-margin-handoff-2026-09`
- Handoff base/head before this document: `3d9cf56fd2b3f4672b1036bfda19573ccbaabcd7`
- Predecessor scheduler PR: #232, merged into `main`
- Current Timing Margin implementation/evidence in #233 remains valid sender-side work.
- Mark ready and merge only after the final defaults/docs commit passes exact-head CI.

This handoff is a follow-up to real-game testing. It does **not** declare the
existing Timing Margin architecture wrong. It changes the investigation
priority because the game-level symptom does not correlate with larger Hold.

Before editing, read:

- `AGENTS.md`
- `SECURITY.md`
- `docs/implementation-user-owned-symmetric-timing-margin.md`
- `docs/hold-frame-model.md`
- `docs/timing-principles.md`
- `docs/rt-dispatch-architecture.md`
- `docs/perf-baselines/2026-09-user-owned-timing-margin-physical-acceptance.md`
- relevant scheduler/telemetry tests for every boundary changed

---

# 1. New real-game observations — treat as product evidence

The following observations came from actual Sky playback and must guide the
investigation:

1. Occasional missing notes still occur with Timing Margin = 0 us.
2. Occasional missing notes still occur with Timing Margin = 800 us.
3. Occasional missing notes still occur with Timing Margin = 1500 us.
4. Missing notes still occur with Base Hold = 1.5 frames and Margin = 1500 us.
   At 60 FPS this is approximately 26.501 ms authored hold.
5. Missing notes tend to recur around the **same position/timestamp in a song**
   rather than looking uniformly random.
6. During real physical playback, the Diagnostics UI has been observed showing
   sender/backend metrics as `Unavailable`, while a pre-call lateness value may
   still appear as `0`.
7. The user now wants Late Down cutoff/tolerance to be explicitly adjustable.

These observations substantially weaken the hypothesis that the primary game
failure is caused only by an authored hold that is a few hundred microseconds
too short.

Do not describe Timing Margin as a general "fix missing notes" control.

The current interpretation should be:

> Timing Margin is sender-side visibility headroom for authored pressed and
> released intervals. It does not prove the game accepted a musical note.

---

# 2. Existing sender evidence — preserve it

The checked-in physical acceptance run
`timing-margin-20260912T154546-6d40ebef` verifies the controlled Windows
sender path, not Sky game consumption.

It proves, for the project-owned sink:

```text
Margin       authored Hold       authored Release Gap
0 us          16,667 us           16,667 us
500 us        17,167 us           17,167 us
800 us        17,467 us           17,467 us
1000 us       17,667 us           17,667 us
3000 us       19,667 us           19,667 us
```

The run also showed sender-side floor anomalies at lower margins and clean
sender-side floors at >=800 us in that run.

Therefore:

- use Timing Margin default **500 us**;
- keep its current range/step unless a later product decision changes it;
- do not silently raise it because of the game symptom;
- do not remove current calibration/recommendation evidence;
- do not claim controlled-sink acceptance proves Sky musical acceptance.

---

# 3. Current root-cause ordering

This is an investigation ordering, not a declaration that any item is already
proven:

1. song-position-specific packet/schedule shape;
2. Sky behavior for batched synthetic keyboard input;
3. Mixed Up+Down physical boundaries;
4. chord batching / multiple keyboard INPUT records in one SendInput call;
5. dense local authored sequence or same-key interaction;
6. late-Down suppression/cutoff under real game load;
7. Hold duration itself.

The fact that the suspected miss tends to recur at the same song location makes
deterministic packet shape more suspicious than purely random host scheduling.

Codex must collect evidence before changing production SendInput semantics.

---

# 4. Locked product decisions for this workstream

1. Existing symmetric authored timing remains:

```text
Hold        = frame_base_hold + timing_margin_us
Release gap = one_frame       + timing_margin_us
```

2. Late Down tolerance is independent from both equations.
3. Late Down tolerance becomes a user-owned persisted setting.
4. Final accepted default is **2000 us**.
5. User-adjustable range: **0..=5000 us**.
6. Normal UI step: **100 us**.
7. `0 us` is valid but should be described as strict/diagnostic behavior.
8. Late Down tolerance is frozen into a prepared/running session.
9. Changing it invalidates/rebuilds prepared playback using the same rules as
   other playback timing settings.
10. It participates in playback/session/fingerprint identity because it changes
    send-vs-drop execution semantics.
11. It must never be added to authored Hold or Release Gap.
12. Calibration remains recommendation/evidence only.
13. Recommended sender margin must use the **selected Late Down tolerance**,
    not a hard-coded 500 us.
14. No automatic setting mutation.
15. Do not change production input batching, scan-code encoding, dwExtraInfo,
    chord shape, or mixed packet semantics in the main path without new
    evidence.
16. Independent acceptance is complete; merge is gated only by final exact-head CI.

---

# 5. Workstream A — fix real-session Diagnostics observability first

This workstream is mandatory and precedes conclusions drawn from cutoff A/B.

## Problem

Current desktop publication builds:

```rust
active.player
    .as_ref()
    .map(NativeDiagnosticsSample::from_player)
    .unwrap_or_else(NativeDiagnosticsSample::unavailable)
```

and the `unavailable()` object fills numeric sender fields with zero.

That allows the UI/data model to blur:

```text
measured zero
vs
no sender sample
vs
backend unavailable
```

The user has observed physical playback in Sky while the Diagnostics panel
reports unavailable sender values and a pre-call value of 0.

The normal Play button prepares with `dry_run = false`, so do not dismiss
this as the "Test playback (no input)" path.

## Required investigation

Find and document the actual cause. Do not paper over the symptom in React.

Prove the active physical session state across:

```text
prepare -> start -> NativeActivePlayback -> player attachment
-> supervisor diagnostics publication -> UI event -> store -> DiagnosticsView
```

Check at minimum:

- whether `active.physical` is true;
- whether `active.player.is_some()` is true;
- session ID binding of diagnostics events;
- whether a stale/unavailable sample can become latest for the active physical
  session;
- whether terminal/starting state transitions can publish an unavailable
  sample over a healthy sample;
- whether DTO/store event handling preserves backend status correctly.

## Target DTO semantics

Do not encode "no sample" as numeric zero.

Recommended contract:

```text
physical_session: bool
player_attached: bool
sender_sample_count: u64
max_sendinput_pre_call_lateness_us: Option<u64>
backend_status: unavailable | healthy | degraded | error
```

If changing the exact DTO shape is cleaner, that is acceptable, but the
semantics are mandatory.

`sender_sample_count` should represent real pre-call sender samples. Prefer a
value derived from trusted worker evidence. It may be computed from the
mutually-exclusive pre-call buckets if that is proven equivalent, or a
dedicated scalar may be added.

Required meanings:

```text
physical=false, player=false
    -> dry-run/nonphysical; sender metrics unavailable

physical=true, player=true, sender_sample_count=0
    -> physical player exists but no sender sample yet

physical=true, player=true, sender_sample_count>0
    -> sender metrics available; max pre-call may legitimately be 0

physical=true, player=false during normal playing
    -> invalid/inconsistent state; surface clearly, do not call it healthy
```

## UI requirements

Diagnostics should expose at least:

```text
Physical session       Yes/No
Player attached        Yes/No
Sender samples         N
Sender backend         Healthy/Degraded/Error/Unavailable
Max pre-call lateness  value or Unavailable/No samples
```

A true measurement of 0 us may display `0 us`.
No measurement must display `No samples` or `Unavailable`, never `0 us`.

Keep the current session-ID guard; fix it if stale event ordering is the root
cause.

Diagnostics must remain observational. Do not enable
`StrictTimingDiagnostic` merely to obtain metrics.

## Investigation result (2026-09-12)

The successful physical-start path creates the native player, stores it in
`NativeActivePlayback`, and attaches it before the supervisor starts publishing
nonterminal diagnostics snapshots. A fresh snapshot from that attached path
therefore cannot be `Unavailable`; the no-player branch is the nonphysical or
inconsistent-attachment path. A failed physical start can temporarily have
`physical_session=true` without a player, but it publishes its terminal start
failure rather than entering the recurring diagnostics monitor. The old
numeric `0` was independently confirmed to be ambiguous: the worker counters
start at zero before the first `SendInput` call, and the old DTO had no sample
count.

The original reported `Unavailable` snapshot cannot be assigned to one of
those paths without its raw session-matched event record. The revised DTO now
publishes physical/player attachment and the sender sample count; it derives
the count from the seven mutually exclusive pre-call buckets and makes the
maximum nullable. The store accepts diagnostics only for the current active
session, and the UI distinguishes unavailable backend, attached-with-no-sample,
and a measured zero. No sender or dispatch policy is enabled to obtain these
metrics.

## Tests

Add tests for:

- physical player attached + zero samples;
- physical player attached + one real sample whose lateness is exactly zero;
- physical player attached + nonzero sample;
- nonphysical/dry-run unavailable;
- impossible physical=true/player=false state is not presented as healthy;
- stale previous-session unavailable snapshot cannot replace a current-session
  healthy snapshot;
- generated DTO types represent unavailable vs zero unambiguously.

---

# 6. Workstream B — user-owned Late Down tolerance

## Naming

Domain/internal field may remain:

```text
down_late_grace_us
```

User-facing label:

```text
Late Down tolerance
```

Suggested explanatory copy:

```text
A Down later than this is dropped instead of being sent late.
```

For 0 us, optionally add:

```text
Strict/diagnostic: any measured lateness beyond the exact target may drop a Down.
```

## Settings schema

Bump the durable settings schema from v4 to v5 using existing adapter
conventions.

In `sky_app_core::settings` add:

```rust
pub const DEFAULT_DOWN_LATE_GRACE_US: u64 = 2_000;
pub const MIN_DOWN_LATE_GRACE_US: u64 = 0;
pub const MAX_DOWN_LATE_GRACE_US: u64 = 5_000;
pub const DOWN_LATE_GRACE_STEP_US: u64 = 100;
```

If `DEFAULT_DOWN_LATE_GRACE_US` remains physically located in
`timing.rs`, avoid duplicate source-of-truth constants; re-export/use one
canonical definition.

Extend:

```rust
PlaybackDefaults {
    hold_frames,
    timing_margin_us,
    down_late_grace_us,
    tempo_scale,
    fps,
}
```

and the playback patch.

Validation:

```text
0 <= down_late_grace_us <= 5000
down_late_grace_us % 100 == 0
```

Explicit invalid API patches must be rejected atomically.

Migration v4 -> v5:

```text
missing field -> 2000
valid existing field (if any compatible legacy source exists) -> preserve
invalid persisted field -> canonical 2000 before writing v5
```

Do not infer the user's value from calibration.

## Materialized timing policy

Normal production materialization must receive the selected user cutoff:

```text
fps
hold_frames
timing_margin_us
down_late_grace_us
```

The authored timing equations stay exactly:

```text
min_hold_us =
    frame_base_hold_us + timing_margin_us

min_release_gap_us =
    frame_us + timing_margin_us
```

Changing only `down_late_grace_us` must leave both fields byte-for-byte
identical.

The sender deadline remains:

```text
latest_allowed_down =
    physical_target + down_late_grace
```

and equality remains admissible; one QPC tick beyond remains rejected.

## Session freeze and identity

The selected cutoff must flow:

```text
settings
-> bootstrap/store
-> PlaybackConfig
-> prepare
-> MaterializedTimingPolicy
-> prepared variant
-> session
-> Diagnostics
-> sender cutoff
```

Changing it while idle invalidates any prepared plan.

Changing settings while a session is running must not mutate that session.

Include selected cutoff in:

- settings fingerprint;
- prepared playback fingerprint;
- cache/decision identity where execution policy identity is currently carried;
- active session metadata.

## UI

Put this under an Advanced timing subsection rather than mixing it into the
primary Hold knob.

Example:

```text
Advanced timing

Late Down tolerance
[ - ]   2000 us   [ + ]

A Down later than this is dropped instead of being sent late.
```

Use the same robust optimistic-step behavior already fixed for Timing Margin:
rapid clicks must preserve every 100-us user step, and failure must resync to
the authoritative setting.

Profile summary does not have to include the cutoff if that makes it noisy, but
the profile detail and Settings summary must show the actual selected value.

Do not call it "engine safety".

---

# 7. Workstream C — recommendation must follow selected cutoff

The recommendation is derived from the selected Late Down tolerance plus the qualified or fallback transport reserve; it is not tied to a fixed 500-us cutoff.

Change it to:

```text
raw_sender_recommendation =
    selected_down_late_grace_us
    + calibrated_or_fallback_transport_reserve_us

recommended_timing_margin_us =
    ceil_to_100us(raw_sender_recommendation)
```

The recommendation remains advisory only.

## Important range interaction

Timing Margin currently has a maximum of 3000 us, while the new cutoff may be
up to 5000 us and qualified transport reserve may make the recommendation
larger still.

Do **not** silently clamp the evidence recommendation to 3000.

Example:

```text
Late Down tolerance       5000 us
Transport reserve          300 us
Sender recommendation     5300 us
Current margin             500 us
Maximum configurable      3000 us
```

The UI should show the true recommendation and make `Use recommended`
unavailable when the value is outside the Timing Margin setting range.

Suggested text:

```text
Recommendation exceeds the current Timing Margin range.
```

This is preferable to hiding unsatisfied headroom.

Changing cutoff should update the recommendation shown for future playback, but
must not mutate an active session or the user's selected margin.

Rename wording where useful from generic `Recommended Timing Margin` to:

```text
Recommended sender margin
```

and explain that it is based on scheduler/SendInput sender evidence, not proof
that Sky accepted a note.

---

# 8. Workstream D — extend the existing RT telemetry trace, do not invent a parallel logger

The project already has:

- `RtTraceRecord`;
- `TelemetryCollector`;
- preallocated `TelemetryMode::Ring`;
- production desktop capacity 1024;
- authored/pre-call/completion timing;
- requested/sent/skipped counts;
- packet kind;
- Win32 error;
- bounded/no-allocation tests.

Use this infrastructure.

## Goal

When the user observes a miss near a repeatable song position, the exported
trace must answer:

```text
Which authored/compiled boundary was this?
Was it Down-only, Up-only or Mixed?
Which key masks were involved?
How many events were requested?
Did SendInput report all inserted?
Was the Down admitted or cutoff?
What were target/pre-call/completion QPC values?
How late was sender entry?
```

## Extend RtTraceRecord minimally

Add fixed-width fields sufficient to identify the physical boundary. Suggested:

```rust
trace_record_index: u32,
compiled_packet_index: u64,
compiled_packet_index_available: bool,
source_action_index: u32,
up_mask: u16,
down_mask: u16,
```

Retain existing:

```text
event_index / kind / polyphony
authored_ticks
effective_deadline_ticks
wake_ticks
send_started_ticks
send_completed_ticks
dispatch_start_error_ticks
requested_count
sent_count
skipped_count
send_attempts
win32_error
flags/outcome
```

Schema 13 also records raw QPC values for the physical target, SendInput
pre-call, SendInput completion, and a rejected-boundary observation. Each raw
value has a fixed-width availability bit; zero is a valid QPC value and must
not be interpreted as missing evidence. A cutoff/backlog miss has an observed
QPC and target QPC, but no pre-call or completion QPC because SendInput was not
attempted for the authored Down.

If `event_index` already exactly equals the source action identity in all
production paths, document and test that before deciding whether a second field
is redundant. Packet identity and masks are still required.

Bump `NATIVE_TELEMETRY_SCHEMA_VERSION` when extending the record. The trace
sequence is distinct from the compiled schedule packet identity; consumers
must check `compiled_packet_index_available` before interpreting that field.

Do not add strings, Vecs, maps, locks, formatting or heap allocation on the RT
path.

## Capacity

Desktop production now preallocates 8192 records. The current x64
`RtTraceRecord` layout is 192 bytes, so the fixed record ring uses 1,572,864
bytes (1.5 MiB) per physical session before container/export overhead. A native
test guards the 2 MiB bound if the record grows later.

The bounded desktop capacity is fixed at 8192 records to preserve typical
full-song traces without allocating based on song size.

Requirements:

- allocation happens before dispatch;
- RT push remains bounded;
- `truncated/dropped` stays explicit;
- do not silently pretend an incomplete trace is complete;
- no change may fail the existing RT no-allocation test.

Do not increase unboundedly based on song size.

## Export/publication

Add a user-accessible or PR-testable way to export the completed session
sender trace after playback/stop/failure.

Preferred form:

```text
JSON/JSONL with:
session ID
song ID
plan fingerprint
timing policy (fps/base/margin/cutoff)
qpc frequency
telemetry schema version
records
attempted/accepted/dropped/truncated
```

The UI does not need a complex trace viewer in this task. A simple
`Export sender trace` action in Diagnostics or a deterministic native command
is sufficient.

Do not write per-event JSON on the RT thread.

## Trace semantic tests

Cover at least:

- Down-only single;
- multi-key chord;
- Mixed Up+Down packet;
- same-key release/retrigger boundary;
- cutoff miss with zero SendInput attempts;
- exact cutoff admitted;
- one tick beyond cutoff rejected;
- partial/zero-progress outcome;
- packet/source/masks agree with the compiled schedule;
- no-alloc production dispatch remains green;
- trace truncation is explicit.

---

# 9. Workstream E — inspect repeatable song-position packet shape

Do not need a specific user song file to build the trace capability, but once a
trace is available, the completion report should explain how to inspect the
suspected timestamp.

The primary classification is:

```text
Down-only single
Down-only chord
Up-only
Mixed Up+Down
same-key retrigger
dense neighboring boundaries
```

The existing planner/compiler behavior must be understood and preserved unless
evidence warrants a later change:

- authored actions are grouped by timestamp/direction;
- releases are canonicalized before activations at a shared timestamp;
- one physical packet may contain both `up_mask` and `down_mask`;
- prepared Win32 INPUT order is all Up events before all Down events;
- one prepared packet is sent via one SendInput call.

Add tests/diagnostic helpers if necessary to print or export this classification
for a song timestamp.

---

# 10. Workstream F — diagnostic A/B strategies are CONDITIONAL, not production changes

Do **not** implement these into the normal production path merely because they
are plausible.

After Workstreams A-D, if a repeatable game miss has a clean sender trace,
Codex may prepare diagnostic/test seams for targeted A/B.

## If misses cluster on Mixed packets

Candidate diagnostic experiment:

```text
current:
    one SendInput([Up..., Down...])

experiment:
    SendInput([Up...])
    controlled micro-gap
    SendInput([Down...])
```

Candidate gaps may include:

```text
0 / 100 / 250 / 500 us
```

This changes physical semantics and must remain behind an explicit
diagnostic/test-only seam until real-game evidence supports it.

## If misses cluster on large Down-only chords

Candidate experiment:

```text
current:
    one SendInput([D1, D2, D3, ...])

diagnostic:
    separate calls and/or very small stagger
```

Again: diagnostic only until evidence.

## If even Down-only singles miss with clean sender evidence

Only then consider A/B of keyboard injection representation, e.g. scan-code
vs a carefully defined alternate representation, or signature behavior.

Do not change production `KEYEVENTF_SCANCODE`, `dwExtraInfo`, scan-code
mapping or one-call chord semantics in this mandatory workstream.

---

# 11. Cutoff A/B plan for real Sky testing

Once Workstreams A-C are implemented and Diagnostics is trustworthy, use the
same song and settings while changing only Late Down tolerance.

Recommended manual game sweep:

```text
500 us
1000 us
2000 us
5000 us
```

Keep Base Hold and Timing Margin fixed for the comparison.

For each run record:

- exact song ID/hash;
- exact plan fingerprint;
- selected cutoff;
- frozen margin/base/FPS/tempo;
- suspected miss timestamp/index;
- sender sample count;
- max pre-call lateness;
- missed Down / hard-late / final cutoff counters;
- exported trace completeness.

Interpretation:

```text
miss disappears as cutoff increases
    -> late-drop/scheduling becomes a strong candidate

miss repeats at same timestamp even at 5000 us, trace shows Down sent fully
    -> cutoff is unlikely to be the primary cause; inspect packet semantics

miss repeats and trace says cutoff miss
    -> scheduler/admission issue, not game consumption
```

Do not infer causality from one run. Repeat the same-song location enough times
to establish whether the position-specific tendency is stable.

---

# 12. Required core tests for editable cutoff

## Settings

- default = 2000;
- 0 accepted;
- 500 accepted;
- 1000 accepted;
- 5000 accepted;
- 1/499/501/5001 rejected as appropriate;
- v4 missing field migrates to 2000;
- invalid persisted value canonicalizes to 2000 in memory and raw v5 file;
- patch remains atomic.

## Timing independence

For fixed FPS/base/margin, run cutoffs:

```text
0 / 500 / 1000 / 2000 / 5000
```

and assert:

```text
frame_base_hold_us identical
min_hold_us identical
min_release_gap_us identical
only down_late_grace_us changes
```

## Sender cutoff

- equality allowed;
- one QPC tick beyond rejected;
- Up-only remains exempt;
- Mixed packet cutoff semantics remain exactly documented;
- rejected Down produces zero musical Down SendInput attempt;
- recovery semantics unchanged.

## Recommendation

With fallback transport reserve 300 us:

```text
cutoff 0    -> recommendation 300
cutoff 500  -> recommendation 800
cutoff 1000 -> recommendation 1300
cutoff 2000 -> recommendation 2300
cutoff 5000 -> recommendation 5300
```

assuming the same fallback reserve.

Qualified calibration uses its actual reserve.

Recommendation >3000 remains visible and is not silently clamped; explicit
`Use recommended` must not submit an invalid patch.

## Fingerprint/session

- cutoff change changes execution identity;
- recommendation-only change does not;
- running session remains frozen;
- prepared plan invalidated after a user cutoff change.

---

# 13. UI acceptance

The UI should remain compact.

Primary controls stay:

```text
Base Hold
Timing Margin
Tempo
FPS
```

Advanced timing adds:

```text
Late Down tolerance
```

Do not expose:

- arbitrary raw QPC values as settings;
- StrictTimingDiagnostic;
- packet split strategy in normal settings;
- scan-code/VK selection;
- duplicate-Down controls.

Diagnostics should clearly separate:

```text
Configuration
Sender availability
Sender measurements
Suppression/transport failures
Trace export
```

Required wording principle:

> No numeric zero may stand in for unavailable evidence.

---

# 14. Physical Windows acceptance after implementation

Run on exact implementation head.

## Existing regression

Re-run the existing 13 physical scenarios at:

```text
FPS             60
Base Hold       1.0f
Timing Margin   800 us
Late tolerance  500 us
Production profile/backend
StrictTimingDiagnostic disabled
```

## Cutoff propagation sweep

Use a physical/test workload sufficient to prove selected values reach the
sender cutoff without altering authored Hold/Gap:

```text
500 / 1000 / 2000 / 5000 us
```

Do not require cutoff=0 to pass an uncontrolled real wall-clock timing run; its
exact semantics belong in deterministic boundary tests.

For each physical sweep point record:

- frozen selected cutoff;
- authored Hold/Gap;
- actual sender cutoff;
- SendInput event stream;
- miss/drop counters;
- terminal invariants.

## Diagnostics physical check

During a real physical run:

- backend must not say Unavailable once the physical player is attached;
- sender sample count must rise after real sends;
- max pre-call lateness must be a real optional measurement;
- no-sample state must not render as measured zero.

Check in raw evidence using the existing
`docs/perf-baselines/...-data/` pattern.

---

# 15. Non-goals

Do not do the following unless later evidence explicitly promotes them:

- raise Timing Margin default above 500 without new evidence;
- lower Timing Margin default below 500 without new evidence;
- couple cutoff into authored Hold/Gap;
- auto-tune cutoff;
- auto-tune Timing Margin;
- split every Mixed packet in production;
- serialize every chord in production;
- emit duplicate Down events;
- switch production keyboard encoding;
- remove the existing one-SendInput atomic packet path;
- change focus/HWND/UIPI/lease safety;
- enable strict timing diagnostics for normal playback;
- add unbounded realtime logs;
- add file I/O or formatting on the realtime worker.

---

# 16. Suggested implementation order

Use small reviewable commits.

Recommended sequence:

1. `fix(diagnostics): distinguish unavailable sender evidence from zero`
2. `feat(settings): add user-owned late Down tolerance`
3. `refactor(timing): thread selected Down cutoff independently`
4. `refactor(calibration): derive sender recommendation from selected cutoff`
5. `feat(desktop): expose advanced late Down tolerance`
6. `feat(diagnostics): export packet-identified sender trace`
7. `test(dispatch): qualify editable cutoff and packet trace invariants`
8. `docs(input): document real-game reliability investigation`
9. exact-head CI
10. physical Windows evidence

The exact split may differ if compilation dependencies require it, but do not
hide speculative input-strategy changes inside these commits.

---

# 17. Acceptance checklist

Code acceptance requires all of the following:

- [ ] Real physical playback Diagnostics no longer conflates unavailable with 0.
- [ ] Root cause of the observed unavailable path is documented/fixed.
- [ ] Physical/player/sample provenance is visible enough to diagnose state.
- [ ] Late Down tolerance is persisted and user-owned.
- [x] Default 2000 us.
- [ ] Range 0..=5000 us, normal step 100 us.
- [ ] Hold and Release formulas remain unchanged.
- [ ] Cutoff changes only send-vs-drop admission.
- [ ] Cutoff is session-frozen and participates in identity.
- [ ] Recommendation uses selected cutoff + transport reserve.
- [ ] Recommendation above 3000 is not hidden/clamped.
- [ ] No automatic setting mutation.
- [ ] RtTraceRecord/export identifies packet/source/masks and true QPC boundaries.
- [ ] Trace remains fixed-width/preallocated on the RT path.
- [ ] Trace truncation/drop is explicit.
- [ ] RT no-allocation test remains green.
- [ ] Exact cutoff equality / one-tick-beyond tests remain green.
- [ ] Existing focus/HWND/UIPI/lease/recovery tests remain green.
- [x] Historical 13-scenario physical matrix remains preserved at its recorded 500-us cutoff; final 2000-us default is not retroactively attributed to that run.
- [ ] Cutoff propagation physical sweep is checked in.
- [ ] PR remains draft pending independent review.

Game reliability is **not** considered solved merely because these boxes pass.
The next decision must use the real-game trace at the repeatable suspected song
position.

---

# Execution record (2026-09-12)

Implementation began at `c4763dac140e62325c8cb8e659a0b225d62a553f` and the
controlled Windows physical acceptance was repeated on exact implementation
head `713d999e0698a77b2a9a11fb6936038d2f22aa47`; the subsequent commit only
archives this run and updates its execution record. Run
`input-reliability-20260912T210236-153c82dc` passed 17/17: the established
13-scenario matrix at 800 us Timing Margin / 500 us Late Down tolerance, plus
the 500 / 1000 / 2000 / 5000 us cutoff propagation sweep with authored Hold
and Release Gap fixed at 17,467 us. The report is bound to 136 continuous
ReceiveOnly sink events; the focus probe received zero keyboard events. Raw
reports, invocation ledgers, ready records, sink/probe logs, runner, and
SHA-256 list are checked in at
`docs/perf-baselines/2026-09-late-down-tolerance-native-acceptance-data/input-reliability-20260912T210236-153c82dc/`.
The earlier run at the implementation commit remains archived alongside it.

This physical run exercises the controlled project-owned sink. It does not
exercise the Diagnostics UI in Sky or identify the user's original
session-matched `Unavailable` event. The implementation now represents
unavailable backend, attached player without samples, and measured zero
separately, with UI regression coverage. The original unavailable observation
still cannot be assigned to one runtime state without its raw session event.
The suspected in-game song position also has not been A/B tested. Export the
sender trace from a real Sky session at that position before drawing a game
reliability or packet-semantics conclusion. Production batching remains
unchanged.

---

# 18. Codex completion report contract

When the mandatory workstreams are complete, report on PR #233 with:

1. exact head SHA;
2. exact commits added after this handoff;
3. changed-file list;
4. diagnosed cause of Diagnostics `Unavailable` during physical playback;
5. final DTO semantics for unavailable/no-sample/measured-zero;
6. final Late Down tolerance schema/default/range/step;
7. final authored timing equations;
8. exact sender cutoff equation;
9. recommendation equation and >3000 behavior;
10. fingerprint/session-freeze behavior;
11. telemetry schema/version and trace fields;
12. trace capacity and memory/boundedness rationale;
13. trace export location/command/UI path;
14. local validation commands/results;
15. exact-head CI run ID;
16. physical acceptance report/raw paths;
17. any deviation from this handoff;
18. any unresolved blocker.

Do not implement or claim a production packet-splitting/chord-serialization
solution unless the trace evidence justifies a separate reviewed change.

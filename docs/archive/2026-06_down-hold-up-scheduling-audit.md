> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: audit lịch sử cơ chế scheduling trước completion-anchor.

# Down / Hold / Up Scheduling Audit

Date: 2026-06-05

Status: independent research pass. This document evaluates the current scheduler and runtime
executor before making any further decision about frame rounding or fixed safety margins.

## 1. Research question

The desired behavior is not simply "schedule a hold longer than one nominal frame".

The player should:

1. preserve authored `down` onset timing as closely as possible;
2. keep each injected key observably down for at least the selected visibility floor;
3. release keys without delaying unrelated onsets;
4. make same-key infeasibility explicit instead of pretending every scheduled down was sent;
5. remain as close to one frame as possible, because globally increasing `min_hold` reduces
   articulation and repeat capacity.

This audit therefore treats rounding and profile margin as the final layer, not the first fix.

## 2. Current execution pipeline

The live path is:

```text
source note
  -> tempo-scaled note intent
  -> same-time same-key deduplication
  -> per-key target hold planning
  -> absolute down/up KeyAction timeline
  -> global up-before-down ordering
  -> PlaybackEngine absolute-deadline dispatch
  -> WinSendInputBackend state filtering
  -> SendInput
```

Important current contracts:

- Authored down timestamps are preserved.
- Exact simultaneous downs are batched as chords.
- Built-in frame-aware profiles currently materialise `hold_us == min_hold_us`.
- Up timestamps are planned ahead of time as `scheduled_down + scheduled_hold`.
- Runtime does not adjust an up deadline using the actual down dispatch time.
- Backend duplicate-down protection skips a down when the same key is still active.

## 3. What is already correct

### 3.1 The pure scheduler has a good basic shape

- Note intent normalisation occurs before same-key analysis.
- Same-time duplicates of the same physical key become one note, not fake repeats.
- Different key lanes do not change each other's planned timestamps.
- Same-time chords are emitted in one down batch.
- Timeline ordering is deterministic.
- Up-before-down ordering prevents a same-key down from being sent before a same-time release.

These are strong foundations and should not be broadly rewritten.

### 3.2 Normal production corpus has large same-key headroom

At tempo `1.0x`, the real-song corpus has:

- zero impossible same-key repeats at 30, 60, and 144 FPS;
- zero schedule invariant violations at 30, 60, and 144 FPS;
- minimum positive same-key interval: 75 ms;
- minimum scheduled same-key up gap:
  - 41.666 ms at 30 FPS;
  - 58.333 ms at 60 FPS;
  - 68.055 ms at 144 FPS.

Therefore, a runtime hold-correctness fix can be introduced without globally lengthening normal
song holds or changing normal authored down onsets.

### 3.3 Sent-side timing is normally clean

Existing real SendInput telemetry shows typical lateness around tens of microseconds and SendInput
call duration around 0.28-0.30 ms. Most scheduled holds therefore execute near their intended
duration.

This supports keeping the one-frame target sharp. It does not prove that the current executor
enforces a one-frame lower bound on every event.

## 4. Correctness gaps

### 4.1 Scheduled minimum hold is not a runtime-enforced minimum

Current scheduling uses:

```text
scheduled_up = scheduled_down + min_hold
```

But runtime hold measured between dispatch starts is:

```text
runtime_hold = min_hold + lateness_up - lateness_down
```

If down is later than up relative to their deadlines, runtime hold becomes shorter than
`min_hold`. Existing telemetry contains a start-to-start hold shortfall as large as about 1.6 ms in
one run. The normal runs were much tighter, but the contract is still not enforced.

This is the main reason that changing `ceil()` or adding a tiny frame ratio cannot solve the entire
problem.

### 4.2 Telemetry records attempted actions, not actions actually sent

`PlaybackEngine._execute_action()` ignores `InputSendResult`.

In a degraded same-key overlap:

```text
down 1
down 2 while key is still active
up 1
up 2
```

the backend sends only one down and one up. Duplicate-down protection skips `down 2`, and the final
up is already released. Telemetry still records all four actions as though they were dispatched.

This makes sender-side evidence overly optimistic exactly in the cases where timing correctness is
most important.

### 4.3 Zero scheduled up gap is ordered, but not observable

When:

```text
same_key_interval == min_hold
```

the scheduler emits an up and the next down at the same timestamp and labels the repeat feasible.
Up-before-down ordering guarantees event order, but it does not create a game-observable UP state.

The two separate SendInput calls create only a small implementation-dependent interval. That is not
equivalent to a measured same-key retrigger guarantee.

This issue is separate from down visibility. Increasing `min_hold` is not the correct general fix.

### 4.4 Degraded impossible repeats have an implicit drop policy

When a same-key interval is below `min_hold`, preserving the previous hold and preserving the next
onset are physically incompatible.

The current degraded policy implicitly preserves the previous note because backend duplicate-down
protection drops the next down. That can be a valid policy, but it is currently an accidental
backend outcome rather than an explicit scheduler/executor decision.

The planner should explicitly report which onset will be dropped or shifted.

### 4.5 Late bursts replay expired state transitions back-to-back

If the playback thread is stalled past several deadlines, the engine sends all overdue actions in
timeline order. A down/up/down sequence may then be injected almost back-to-back and sampled only in
its final state by the game.

This is unavoidable information loss after a sufficiently long stall, but the recovery policy
should be explicit. Replaying expired pulses is not always better than deliberately dropping an
expired note.

### 4.6 Documentation and live profile values have drifted

The working tree currently declares frame ratios `1.0 / 1.01 / 1.02` for
`local_precise / balanced / audience_safe`, while `timing-profile-frame-model.md` still describes
different values.

Any later margin decision must use resolved live policy values and update the source-of-truth
documentation in the same change.

## 5. Best-fit runtime model

The best model for the stated objective is an onset-priority, per-key release-eligibility executor.

### 5.1 Keep downs on the authored absolute timeline

Do not shift normal down timestamps to protect holds. Down onsets carry the musical rhythm and should
remain the primary deadlines.

### 5.2 Anchor the minimum hold to confirmed down dispatch

`SendInput` returns the number of events inserted into the input stream. Therefore, completion of a
successful down call is a conservative confirmation point: the down has been inserted before that
time, while a future up cannot be inserted before its call begins.

For each active key:

```text
release_target = scheduled_down + target_hold
release_not_before = down_send_completed + min_hold
effective_release = max(release_target, release_not_before)
```

This preserves a lower bound between confirmed down insertion and the start of up insertion without
adding a large fixed margin to every profile.

With current measurements, the adaptive extension is usually around one SendInput call duration,
roughly 0.3 ms, instead of a fixed 10-20% frame margin.

### 5.3 Defer releases per key without blocking unrelated downs

A naive wait until `release_not_before` inside the current sequential action loop would delay every
later action, including unrelated downs. That would damage rhythm.

The executor instead needs:

- a queue of authored down deadlines;
- a queue of pending per-key releases;
- active-key runtime state containing confirmed down completion;
- the ability to defer one key's release while continuing to dispatch other keys' due downs.

This is the important architectural requirement. Runtime hold enforcement must not become a global
blocking wait.

### 5.4 Make same-key conflict policy explicit

For a next down on an already active key:

- if the key can be released before the down while preserving the runtime hold floor, release then
  send the down;
- otherwise classify the onset as physically infeasible;
- `strict` rejects before playback where predictable;
- `degraded` explicitly chooses and reports a policy such as preserving the previous note or
  prioritising the new onset.

Do not rely on backend duplicate filtering as the conflict policy.

### 5.5 Treat repeat up visibility separately

One-frame down visibility and same-key UP visibility are different constraints.

A zero-duration or tiny UP state can preserve event ordering while still failing to retrigger in the
game. This should first become:

- an explicit runtime/schedule metric;
- a diagnostic or strict feasibility rule backed by the existing same-key experiments.

It should not be reintroduced as a global hold increase. At normal tempo, the real corpus already
has ample up gap.

### 5.6 Record conservative runtime evidence

Telemetry should record:

- backend `sent` and `skipped_duplicates`;
- dispatch start and completion for each action;
- per-key confirmed hold lower bound:

  ```text
  up_send_started - down_send_completed
  ```

- scheduled and runtime same-key up gap;
- explicitly dropped or shifted onsets;
- late-burst recovery decisions.

This evidence is more useful than only recording action-attempt start times.

## 6. Why this is better than increasing min_hold

Increasing `min_hold` globally:

- lengthens every note;
- reduces same-key repeat capacity;
- can make articulation worse;
- still does not guarantee runtime hold if differential lateness is larger than the margin;
- does not solve skipped degraded downs or invisible zero-gap repeats.

The proposed runtime guard:

- leaves normal down onset timing unchanged;
- extends only releases that would otherwise violate the actual hold floor;
- usually adds only the measured dispatch uncertainty;
- isolates the extension per key;
- makes true infeasibility observable.

This is the closest practical design to a real one-frame minimum without making every note
unnecessarily longer.

## 7. Rounding and margin decision

Rounding should be evaluated only after runtime hold enforcement exists.

Current formula:

```text
frame_us = ceil(1_000_000 / fps)
hold_us = ceil(frames * frame_us)
```

Observations:

- `ceil()` is correct for preventing a nominal 1.0-frame value from falling below the mathematical
  frame period.
- Double rounding contributes at most about 1 us in the audited FPS/ratio set.
- This rounding error is negligible compared with SendInput duration, runtime lateness, and game
  frame-time variation.

Recommended order:

1. enforce runtime confirmed hold;
2. improve telemetry;
3. measure confirmed hold lower-bound distribution and in-game registration;
4. only then decide whether `1.0 frame` needs a small game-side margin.

If a margin is still required, it should cover measured game/frame variance, not compensate for an
executor contract that can be enforced directly.

## 8. Implementation phases

### Phase A - observability first

- Capture `InputSendResult` in the engine.
- Record sent/skipped actions and dispatch completion times.
- Report confirmed hold lower bounds.
- Add a regression test proving skipped duplicate downs are not reported as sent.

### Phase B - non-blocking per-key release guard

- Add runtime active-key state with down completion and release eligibility.
- Defer ineligible releases per key.
- Keep unrelated authored downs dispatchable.
- Test asymmetric lateness and simultaneous chords.

### Phase C - explicit same-key conflict and stale-event policy

- Remove accidental reliance on backend duplicate-down filtering.
- Define degraded choice for impossible repeats.
- Define behavior for expired down/up pairs after a long stall.
- Add schedule/runtime diagnostics for zero or insufficient up visibility.

### Phase D - frame rounding and margin experiment

- Keep `local_precise` at the sharp one-frame candidate.
- Compare one-frame registration before and after runtime enforcement.
- Tune a game-side margin only if misses remain under a confirmed runtime hold floor.

## 9. Audit evidence

- Full suite: `216 passed`.
- Focused timing/playback/scheduler suite: `76 passed`.
- Corpus parsed: 110 songs, 76,317 notes.
- Real-song corpus at `1.0x`: no impossible repeats or invariant violations at 30/60/144 FPS.
- At `3.0x`, current one-frame-down-only feasibility becomes optimistic:
  - 60 FPS: five real songs have a scheduled same-key up gap below one frame, while current
    `impossible_same_key_repeats` remains zero;
  - 30 FPS: 65 real songs have an up gap below one frame and 25 impossible repeats.
- Boundary simulation:
  - interval `< min_hold`: second down is skipped by backend;
  - interval `== min_hold`: second down is sent after a same-timestamp up, but scheduled UP
    visibility is zero;
  - interval `> min_hold`: scheduled positive up gap exists.

## 10. Decision

Do not increase global `min_hold` yet.

The next correctness improvement should be runtime enforcement of the existing one-frame hold
against confirmed down dispatch, implemented with non-blocking per-key release deferral and truthful
backend-result telemetry.

After that change, a new in-game one-frame experiment can determine whether any remaining margin is
needed for the game itself rather than for sender/runtime uncertainty.

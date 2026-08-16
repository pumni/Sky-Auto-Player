# 05 — P1: Timing Boundaries and Telemetry Semantics

## 1. Goal

Make every timing field describe the boundary actually measured, while keeping measurement overhead out of the production precision path.

The system must not optimize against a mislabeled timestamp.

---

# 2. Canonical sender-side boundaries

Use these names in new Rust code and normative docs:

```text
physical_target_qpc
final_admission_qpc
sendinput_completion_qpc
```

Definitions:

### `physical_target_qpc`

Absolute QPC time at which the selected physical operation is intended to begin sender-side dispatch.

### `final_admission_qpc`

One QPC sample taken after final command and Down target/focus checks, immediately before the final lease classification and transport call.

This is the production start boundary.

### `sendinput_completion_qpc`

QPC sample taken immediately after the `SendInput` return path, before observer/health work.

---

# 3. What `final_admission_qpc` is not

Do not call it:

```text
exact SendInput syscall start
kernel insertion timestamp
OS delivery timestamp
game input timestamp
note onset
```

Between `final_admission_qpc` and the actual API call there is still bounded code, including final lease classification, Rust call frames, and `SetLastError(0)`.

This distinction matters because a future developer might otherwise subtract the wrong “SendInput cost” from authored targets.

---

# 4. Production metrics

Primary signed start error:

```text
dispatch_start_error_ticks = final_admission_qpc - physical_target_qpc
```

Sender call-envelope duration:

```text
admission_to_completion_ticks =
    sendinput_completion_qpc - final_admission_qpc
```

Completion residual:

```text
target_to_completion_ticks =
    sendinput_completion_qpc - physical_target_qpc
```

Release hold evidence:

```text
release_not_before = down_completion + effective_min_hold
release_due = max(authored_up, release_not_before)
```

All are sender-side evidence only.

---

# 5. Compatibility names

External report/schema names may remain for compatibility, but their mapping must be explicit:

```text
send_started_ticks   -> final_admission_qpc
send_completed_ticks -> sendinput_completion_qpc
send_duration_us     -> admission_to_completion
```

If changing the public schema would cause unnecessary migration risk, keep the old keys and update `TimingSemantics`/docs rather than duplicating QPC samples.

Do not maintain two physical samples just to satisfy an old field name.

---

# 6. Optional diagnostic syscall-entry probe

Add a benchmark/test-support-only mode that can capture:

```text
sendinput_call_qpc
```

at the narrowest practical point immediately before `SendInput`.

Then derive:

```text
admission_to_call = sendinput_call_qpc - final_admission_qpc
call_to_completion = sendinput_completion_qpc - sendinput_call_qpc
```

Rules:

- disabled in normal production by default;
- observation only;
- never used to move a target;
- never used to learn a future lead;
- benchmark its own QPC-read overhead;
- do not retain permanently in production unless a later evidence-backed decision explicitly approves it.

A feature/test flag is preferable to another runtime branch in shipping code if practical.

---

# 7. Wake evidence

`WaitResult::wake_qpc` is the authoritative sample produced at the deadline handoff.

Observer may derive:

```text
wake_error = wake_qpc - physical_target_qpc
wake_to_admission = final_admission_qpc - wake_qpc
```

Do not take another QPC sample just to create these two fields.

For a plan that was already overdue before entering a wait, `wake_qpc` may be absent or classified separately. Do not fabricate a wake timestamp equal to admission time.

---

# 8. Signed values stay signed

Start/completion residuals must retain sign.

Do not convert:

```text
-80 µs early
```

into:

```text
80 µs error
```

inside authoritative telemetry.

Absolute error can be calculated as an additional reporting statistic, but it must not replace signed evidence.

---

# 9. No controller feedback

These values are measurements:

```text
wake error
start error
admission-to-completion
completion residual
Raw Input calibration delivery delta
```

They are not controller inputs for authored note-on timing.

Explicitly prohibited:

```text
next_target -= last_send_duration
next_target -= EMA(completion_residual)
PID(start_error)
lead = p99(raw_input_delivery)
```

A persistent positive residual should trigger investigation/benchmarking, not automatic schedule mutation.

---

# 10. Observer ownership

The producer enqueues raw ticks/masks/status.

The observer owns:

- tick→microsecond conversions;
- p50/p95/p99 summaries;
- lateness buckets;
- health windows;
- compatibility record materialization;
- JSON/report structures;
- warning/degraded state publication;
- approximate queue-depth diagnostics.

The observer must receive its own `QpcClock` conversion context and fixed policy at startup.

---

# 11. Remove/deprecate misleading fields

Review all snapshot/telemetry fields containing words such as:

```text
actual
onset
ready
send_start
latency
```

For each field, either:

1. document the exact sender-side boundary;
2. rename internally and preserve a compatibility alias externally; or
3. deprecate if it has no reliable interpretation.

In particular, production does not sample `dispatch_ready_qpc` today. Do not re-enable it on every send just to populate `core_post_send_duration_us`.

Use diagnostic builds/benchmarks for post-send-ready measurements.

---

# 12. Raw Input calibration boundary remains separate

Calibration can measure:

```text
SendInput completion -> app-owned WM_INPUT handler receipt
```

That evidence may inform the already-defined hold margin only.

It does not prove:

- game-side polling time;
- render frame;
- audio onset;
- network propagation.

Do not merge calibration timestamps into the playback note-on controller.

---

# 13. QPC conversion rules

Keep frequency captured once per worker.

Prefer tick-domain comparisons:

```text
now_ticks >= target_ticks
completion_ticks >= start_ticks
lateness_ticks > threshold_ticks
```

Convert fixed configuration thresholds to ticks at startup.

Convert observations back to microseconds in observer/reporting.

Do not repeatedly roundtrip:

```text
QPC ticks -> µs -> QPC ticks
```

on the physical path.

---

# 14. Required tests

1. `send_started_ticks` compatibility alias equals `final_admission_qpc` exactly.
2. No second QPC sample is added solely to preserve compatibility naming.
3. Signed start error can be negative/zero/positive.
4. Missing completion after a reported physical success is terminal.
5. Completion preceding final admission is terminal.
6. Diagnostic `sendinput_call_qpc` ordering, when enabled:

```text
final_admission_qpc <= sendinput_call_qpc <= completion_qpc
```

7. Normal production test seam proves diagnostic call sample is not captured.
8. Observer can fully materialize required compatibility telemetry from compact raw evidence.
9. Telemetry-off production still preserves all correctness state and timing required for release floors.

---

# 15. Acceptance

This phase is complete when a reviewer can answer “what physical event does this timestamp represent?” for every timing field without reading implementation guesses, and when enabling/disabling telemetry cannot change the physical target or healthy send authorization.

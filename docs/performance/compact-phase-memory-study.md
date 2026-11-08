# Issue #640 — phase-separated private commit vs Rust requested-layout accounting

**Draft diagnostic, offline-only, Windows-only, test-support only.** This
experiment does not change the live sender, production planner, frozen
prepared packet format, `SendInput`, Donate, or the default representation.

## Why another test is justified

The completed #641 compact-masks vs legacy compact full process A/B on
`b36188d198f35597f019a904f6dbdfa70b02bcc9` (2,440 process trials,
1,220 matched pairs) and its separate **read-only** incremental analysis
have already established:

- In synthetic 1,000 generations × width 15 warm, mask-reserved compact
  used **80,000 B less** retained INPUT arena and **three fewer** Rust
  alloc/realloc calls, yet had **+602,112 B** median *held PrivateUsage*.
  All 5/5 paired **incremental** PrivateUsage deltas were positive.
- At width 14 warm, reserve fell **160,000 B** but median incremental
  PrivateUsage rose **+479,232 B**, positive in 4/5 pairs.
- Sparkle warm had +188,416 B held PrivateUsage even though its incremental
  median was −4,096 B; its positive **before-prepare** offset matters.
- All 1,220 raw paired identities reconciled exactly:
  held delta = before-prepare delta + incremental delta.

This is **not** proof of allocator size-class behavior or fragmentation.
Both snapshots are whole-process Win32 counters, and the existing Rust
allocator count lacks *released layout bytes* or post-drop process memory.

## Added instrumentation in existing offline example only

`rt_prepared_process_probe.rs` now records three *plain, nonallocating*
Win32 `GetProcessMemoryInfo` samples per independent child process:

1. `before_prepare_process` — after source compilation and same-variant
   warmup (if requested), immediately before measured preparation.
2. `while_owner_held_process` — with the chosen stream AND reset coordinator
   still live (after measured build/reset), before JSON serialization.
3. `after_owner_drop_process` — immediately after that test-support method
   returns and its stream/reset owners drop, before constructing JSON.

The original `prepare_reset_ns` timing still ends before the held sample.
`before_prepare_process` and `while_owner_held_process` preserve their
previous JSON field names and Win32 definitions. A new
`ProcessMemorySnapshot` struct avoids intermediate `serde_json::Value`
heap allocations *during* the observation window.

The counted `System` Rust global allocator now records a separate
cumulative `released_layout_bytes` counter, incrementing by the old layout
size when `realloc` succeeds and by freed layout size on `dealloc`.
Together with the existing requested new-layout bytes, this yields:

`rust_net_requested_layout_bytes = Δrequested_bytes − Δreleased_layout_bytes`

The probe emits allocator snapshots `before_prepare_allocator`,
`while_owner_held_allocator`, and `after_owner_drop_allocator`,
plus the signed Rust net requested-layout changes from before→held and
before→postdrop. **These are Rust requested allocation layout bytes, NOT
OS-committed bytes, real heap usable blocks, private RSS or retained
Win32/native/CRT allocations.** Additional relaxed atomics can affect
prepare timings; compare only paired variants measured with this *same*
instrumented executable, never directly against earlier #641 prepare
timings as though instrumentation cost were zero.

No packets, INPUTs, coordinator, owning streams or reserve policies change.
The previous `scripts/qualify_compact_mask_hint_ab.ps1` and accepted baseline
`scripts/qualify_compact_prepared_ab.ps1` are **unchanged**. Existing
`schema:1` fields remain readable; the phase fields are additive.

## Gate and sequence

Do **not** merge on the strength of old #641 results. Exact draft SHA/tree
must pass hosted Windows CI/static/security/signing and new Windows unit
tests. Native measurements may run in parallel with CI but must be
qualified on the same SHA before any merge.

- **Smoke:** run `scripts/qualify_compact_mask_hint_ab.ps1` with
  `-Scope smoke -Pairs 5` on an existing clean **detached validation
  worktree**, source HEAD pin, and fresh TEMP evidence. Expect **11
  workloads, 220 separate child process JSON and 110 matched pairs**,
  canonical 112/110/2 corpus and exact histogram.
- **No immediate full rerun:** initially analyze the 220 raw JSON and
  confirm the new phase/allocator conservation. If the after-drop and
  net-requested signals help isolate an observable mechanism and all
  parity checks hold, consider full 2,440 child-process/1,220 matched
  pairs separately.
- Preserve all original #641 and earlier evidence. No local original
  `main` changes, no SendInput, no production playback, no new worktree,
  no commit from Windows agent.

## Postprocessing (per child, then per matched pair)

For each counter (`private_commit_bytes`, `working_set_bytes`):

- `prepare_increment = held − before`
- `postdrop_change = postdrop − before`
- `drop_reclaim = held − postdrop`

For each pair, subtract **legacy compact** from **compact-masks** at
each point. Required exact equalities for each independent pair:

`held_pair_delta = before_pair_delta + prepare_increment_pair_delta`

`postdrop_pair_delta = before_pair_delta + postdrop_change_pair_delta`

`prepare_increment_pair_delta = postdrop_change_pair_delta + drop_reclaim_pair_delta`

And for Rust requested layout bytes per child:
`rust_net_before_to_held = (held.requested − before.requested) −
(held.released − before.released)`; similarly before→postdrop.

Check that cumulative allocator counters do not decrease within a
process. Report all five paired deltas with AB/BA and cold/warm for
synthetic width 14/15 and Marble Machine/Sparkle. Summarize
private commit, working set, Rust live-requested-layout changes, and
arena used/reserved bytes **separately**. Do not delete or trim
negative observations/outliers.

### Interpretation decision tree

- If held PrivateUsage increases but **post-drop PrivateUsage stays
  elevated** while Rust net-live requested layout bytes return toward
  baseline, that is *consistent with* allocator/heap retention or other
  process overhead; **not causal proof**.
- If post-drop PrivateUsage and net requested layout bytes fall
  together, the excess was more closely associated with still-live
  Rust allocation request sizes; determine which owners only in an
  additional controlled offline test.
- If before-prepare offsets dominate while incremental/postdrop signals
  are small, note process baseline differences rather than declaring an
  optimization failure of the prepared stream itself.
- A Win32 process counter cannot identify which allocation site or
  native CRT subsystem causes the change; another isolated allocation
  trace would be needed for causal attribution.

## Status

**Unmeasured new diagnostic.** The earlier #641 native full A/B and its
read-only paired decomposition remain authoritative for the uninstrumented
code. This draft does not make any speed, RAM, RSS, production sender,
receiver or gameplay claims.

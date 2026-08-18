# 03 — P0: Overdue / Stall Fidelity Policy (superseded design)

This working-note file records the current production policy. The earlier
boolean physical-boundary abort policy is obsolete: ordinary deadline misses
after startup are recoverable, while the no-catch-up guarantee remains
mandatory.

## 1. Problem

A scheduler stall can make several authored Down boundaries overdue at once.
Sending them in a tight loop changes the authored rhythm:

```text
authored:  A ---- 8 ms ---- B ---- 8 ms ---- C
stall:                          40 ms
wrong:     send A, send B, send C as fast as the worker can run
```

The recovery policy must preserve authored absolute timestamps, never replay a
Down backlog, and keep a started Production song alive when lateness alone is
the problem.

## 2. Exact Down authorization

Do **not** add adaptive rebase, adaptive lead, or a catch-up controller.

After a successful musical Down commit, the worker enters a Down-only
`AwaitingFuture` state. The exact next Down-bearing boundary may become
`FutureAuthorized` only after the frozen boundary is observed with
`physical_target_qpc > now`. The fixed `PhysicalBoundaryStamp` contains the
prepared batch/packet identity, source action, Up/Down masks, and physical QPC
target; QPC equality alone is not authorization.

The stamp survives waiter-entry latency and a same-boundary
`Continue`/replan. It is invalidated by a changed packet or target, playback
epoch/rebase, manual/focus pause, coordinator invalidation, or completion of
the stamped boundary. `wait_result.is_some()` is transport evidence, not the
musical authority. Up-only sends never arm or clear this state.

## 3. Production miss semantics

When a Down-bearing boundary is due and has no exact authorization, classify it
as a missed musical Down. Before the first successful musical Down commit this
is still a startup-terminal failure with zero musical input. After startup:

- Down-only: make zero `SendInput` calls and commit the frozen authored frame
  with its Down mask marked `DroppedExpired`.
- Mixed: send only one prebuilt Up-only safety packet, require complete
  transport confirmation, then commit the Up release and missed Down mask from
  the frozen commit token. A partial, zero, skipped, or uncertain Up is
  terminal and requires cleanup.
- Up-only: send the owned Up normally even when its authored target is late;
  it is exempt from the musical no-catch-up rule and does not alter Down
  authorization state.

The coordinator must perform a typed deadline-miss commit. Advancing an index
without ownership accounting is not a valid recovery. Dropped Down generations
never become active, and their later authored Ups become stale/no-op.

## 4. Hard-late sender cutoff

The trusted sender keeps the fixed 20 ms Down hard-late cutoff and the
authoritative pre-call QPC check. If an authorized Down crosses that cutoff,
`SendInput` is not called. In Production after startup, the typed result follows
the same missed-Down recovery path. StrictTimingDiagnostic may record the
evidence and remain terminal for qualification.

This cutoff is a syscall-safety boundary, not a session-fatal policy. QPC
failures, target/focus integrity failures, lease violations, ownership
corruption, partial transport, and cleanup failures remain terminal.

## 5. No rebasing or replay

The worker never shifts the epoch, changes `scheduled` or `physical_target`,
adds recovery delay, spaces a replay burst, retries a Down, or uses an EMA/PID
controller. After a miss, the next authored future boundary is evaluated at
its original absolute target.

For example:

```text
authored:  A @ 100, B @ 110, C @ 120, D @ 130, E @ 180 ms
stall:                    worker resumes at 150 ms
physical:  A, no B Down, no C Down, no D Down, E @ 180 ms
```

The result is neither `B C D` at 150 ms nor a fatal session solely because
those Down deadlines were missed.

## 6. Required tests

The deterministic matrix must cover:

1. normal future Down authorization and send;
2. future observation followed by a stall before waiter entry;
3. deadline handoff followed by same-boundary `Continue`/replan;
4. authorization isolation across a different boundary, including equal QPC;
5. unobserved overdue Down with zero Down syscalls and a missed commit;
6. three overdue Downs with no burst and a later future Down;
7. overdue Up-only release with unchanged Down state;
8. overdue Mixed recovery as Up-only;
9. failed/partial recovery Up as terminal cleanup;
10. later Up for a dropped Down as stale/no-op;
11. authorized small lateness still sendable;
12. trusted pre-call beyond 20 ms with zero Down syscall and Production recovery;
13. first musical Down hard miss as startup-terminal;
14. mid-song hard miss as nonterminal;
15. focus/manual pause invalidation;
16. no two unauthorized overdue Down-bearing boundaries sent;
17. arbitrary injected stalls without timestamp mutation or catch-up;
18. no-allocation coverage for missed and hard-late Production paths.

## 7. Bounded diagnostics

The precision path records scalar counters only:

```text
missed_down_boundaries
missed_down_keys
missed_backlog_boundaries
missed_hard_late_boundaries
late_authorized_boundaries
deadline_authorization_reuses
max_missed_lateness_ticks
```

Optional last-sample fields remain fixed-width. No formatting, heap
allocation, schedule scan, or telemetry construction is performed before the
recovery decision. A Production HUD may surface a lightweight missed-note
count; it must not show a fatal playback modal for a recoverable miss.

## 8. Acceptance

After an injected stall:

```text
no two distinct unauthorized overdue Down-bearing boundaries produce Down
SendInput calls without an intervening exact FutureAuthorized stamp
```

Safety Ups remain sendable, future authored timestamps remain unchanged, and
Production resumes at the next valid future boundary. Only genuine QPC,
transport, ownership, integrity, lease, focus, or cleanup failures terminate
the session.

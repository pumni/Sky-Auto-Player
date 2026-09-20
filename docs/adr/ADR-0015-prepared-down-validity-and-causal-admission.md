# ADR-0015: Prepared-Down Validity and Causal Admission

Status: Accepted.

Date: 2026-09-20.

This ADR is the Phase 5 refinement of the accepted prepared dispatcher in
[ADR-0014](ADR-0014-deterministic-prepared-dispatch.md). It supersedes only
the obsolete normal prepared overdue-Down, backlog, and sender-cutoff
wording in the earlier decisions. It does not reopen scheduler, admission,
transport, foreground-proof, or immutable-packet decisions accepted by
Phases 2–4.

## Decision

### 1. Startup integrity admission is control-plane only

After the existing exact Sky target HWND resolution and validation, and before
focus verification, native session construction, or `arm`, startup calls the
existing read-only `target_integrity_compatibility(hwnd)` primitive:

| Result | Startup action |
| --- | --- |
| `Compatible` | Continue startup. |
| `Mismatch` | Fail startup with `target_integrity_mismatch`; do not arm or start physical dispatch. |
| `Unknown` | Continue startup; never relabel the result as a UIPI mismatch. |

Only `Mismatch` maps to `target_integrity_mismatch`. Query failure, an invalid
HWND/PID, a denied limited-information process handle, or an unreadable token
integrity label produces `Unknown`. No new status surface is invented for the
bounded `Unknown` diagnostic.

The primitive requests only the already-approved read-only rights:
`PROCESS_QUERY_LIMITED_INFORMATION` for the target process and `TOKEN_QUERY`
for token integrity labels. It does not read process memory, inject input,
open a debug/VM handle, or acquire a new privilege. The query is made once on
the startup/control-plane path. It is forbidden on the realtime worker,
prepared final admission, per-note path, or sender hot path.

The production chain is therefore:

```text
prepared request/variant
  -> exact target HWND resolution/validation
  -> target_integrity_compatibility(hwnd)
  -> focus verification
  -> NativeDispatchSession construction
  -> supervisor-owned session arm
```

### 2. Effective authored hold validity

The authored schedule is validated before worker start with the materialized
hold floor:

```text
effective_min_hold = frame_base_hold + Timing Margin
authored_up >= authored_down + effective_min_hold
```

Timing Margin is included once in `effective_min_hold`; it is not added again
as a normal recovery grace. For every prepared Down with a paired same-key
authored Up, preparation freezes the smallest static hold slack:

```text
hold_slack = authored_up - authored_down - effective_min_hold
prepared_sender_cutoff = physical_target + hold_slack
```

This cutoff is an authored hold-validity boundary owned by the immutable
prepared packet. The sender compares its true pre-call QPC against it before
`SendInput`. It is not a `PhysicalTimingWindow`, not a `PhysicalTimingGuard`
decision, not a scheduler-lateness allowance, and never moves the authored
target.

An unpaired Down is represented by `NoPairedRelease`: it has no invented finite
sender cutoff. It nevertheless requires the exact causal future-authorization
proof below. Up-only safety traffic has no Down authorization requirement.

### 3. Causal future authorization

Every prepared Down-bearing boundary carries an exact frozen identity stamp:
prepared batch/frame identity, packet identity and masks, physical target QPC,
and target generation. Before the target, the worker must observe that exact
stamp while the target is strictly in the future. A sample at the target or
after it does not authorize the Down.

The proof survives waiter-entry latency and a same-boundary `Continue`/replan.
It is cleared by a changed prepared plan or target, epoch/focus/pause reset,
explicit suspension invalidation, or completion/missed commit. A kernel wait
result, a wake timestamp, or a supervisor focus hint is not the proof.

If a due Down lacks the exact proof, it is committed as a missed authored
boundary and never retried or emitted as a catch-up burst. This is the causal
`UnobservedBacklog` case, not a dynamic physical-window policy. If a paired
Down reaches the sender after its static authored cutoff, the sender returns
`DownExpiredBeforeSend` and the same missed-boundary recovery is used.

### 4. Transactionally consistent normal recovery

Normal prepared miss recovery has one shared bounded Up-prefix emitter and one
frozen same-key continuation/commit path. A Mixed frame emits only its
canonical immutable Up prefix before committing the missed Down; it does not
rebuild a payload or invoke `PhysicalTimingWindow`/`PhysicalTimingGuard`
policy. Up-prefix transport must be complete and confirmed, or playback fails
closed.

Recovery never retimes a later frame, rebases the timeline, retries a Down, or
creates a stale catch-up burst. `DroppedExpired` retains its frozen
same-key continuation semantics. Completion QPC remains sender/transport
evidence and does not become a later scheduling deadline.

### 5. Foreground and transport evidence boundaries

The Phase 4 fresh foreground proof, lifecycle invalidation/reacquisition, and
final atomic target/focus gate remain accepted. The startup integrity check is
a separate control-plane admission hint; it does not replace foreground
proof and does not claim to close the residual query-to-`SendInput` race.

The true sender-owned `pre_call_qpc` is the boundary for authored sender
validity. `SendInput` completion proves only the transport result and sender
timing. Neither boundary proves game polling, rendering, Raw Input delivery,
audio onset, or gameplay observation.

## Consequences and verification obligations

- Compatible, target-higher Mismatch, target-lower Compatible, and query-failure Unknown are deterministic test cases.
- Startup maps only Mismatch to `target_integrity_mismatch`; Unknown continues without that label.
- Source audits must show no integrity/token query in realtime worker or prepared sender code.
- The normal prepared path retains immutable packets, one transport transaction per physical boundary, bounded allocation-free sender behavior, and all accepted Phase-3/4 scheduling/admission semantics.
- ADR-0010 through ADR-0013 remain historical qualification records. ADR-0014 remains accepted except for the narrowly identified normal prepared wording refined by this ADR.

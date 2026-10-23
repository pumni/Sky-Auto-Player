# ADR-0015: Prepared-Down Validity and Causal Admission

Status: Accepted.

Date: 2026-09-20.

This ADR is the Phase 5 refinement of the accepted prepared dispatcher in
[ADR-0014](ADR-0014-deterministic-prepared-dispatch.md). It supersedes only
the obsolete normal prepared overdue-Down, backlog, and sender-cutoff
wording in the earlier decisions. It does not reopen scheduler, admission,
transport, foreground-proof, or immutable-packet decisions accepted by
Phases 2–4.

The startup integrity admission below remains current. The runtime cutoff and
recovery design below is historical; current Down/effective-min-hold policy is
in ADR-0017 and lifecycle/lease policy is in ADR-0018.
Original source: [ADR-0015 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0015-prepared-down-validity-and-causal-admission.md).

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

At this decision date, preparation required
`authored_up >= authored_down + effective_min_hold`, where
`effective_min_hold = frame_base_hold + Timing Margin`. A paired Down froze
`hold_slack = authored_up - authored_down - effective_min_hold`; its
historical sender cutoff was `physical_target + hold_slack`. This was an
authored-validity test, not a physical guard or lateness grace. An unpaired
Down had no finite cutoff but still required causal authorization; Up-only
safety traffic needed no Down authorization.

### 3. Causal future authorization

The historical decision required a frozen identity stamp (prepared
batch/frame, packet and masks, target QPC, target generation) observed strictly
before target. Wait results, wake timestamps, and supervisor focus hints did
not prove it. The proof survived waiter-entry latency/replan and cleared on
plan/target, epoch, focus, pause, suspension, completion, or missed-commit
changes. A Down without proof became `UnobservedBacklog`; a paired Down past
its static cutoff became `DownExpiredBeforeSend`. Both were missed once,
never retried or caught up.

### 4. Transactionally consistent normal recovery

Miss recovery used a shared bounded Up-prefix emitter and frozen same-key
continuation/commit. A Mixed frame emitted only its canonical immutable Up
prefix before committing the missed Down; complete confirmed Up transport was
required or playback failed closed. It did not rebuild payloads, invoke
physical-window/guard policy, retime later frames, rebase, retry, or catch up.
`DroppedExpired` retained its frozen same-key continuation semantics.
Completion QPC remained sender/transport evidence, not a later deadline.

### 5. Foreground and transport evidence boundaries

Fresh foreground proof, lifecycle invalidation/reacquisition, and the final
atomic target/focus gate remained accepted. Startup integrity was a separate
control-plane hint and did not close the query-to-`SendInput` race. The
sender-owned `pre_call_qpc` measured the
historical authored-validity boundary; completion proved only transport and
sender timing. Neither proved game polling, rendering, Raw Input, audio onset,
or gameplay observation.

## Consequences and verification obligations

- Compatible, target-higher Mismatch, target-lower Compatible, and query-failure
  Unknown remain startup test cases. Only Mismatch maps to
  `target_integrity_mismatch`; Unknown continues without that label. Integrity
  and token queries stay off the realtime/sender path.
- Immutable packets, one transaction per boundary, bounded allocation-free
  sender behavior, and the Phase 3/4 accepted architecture remain part of this
  record. ADR-0010 through ADR-0013 are historical; current Down and lifecycle
  semantics are defined by ADR-0017/0018.

# ADR-0001: Packetized Native Input Dispatch

Status: accepted historical record; superseded for the current runtime by
[ADR-0010](ADR-0010-physical-timing-feasibility.md) and
[`docs/rt-dispatch-architecture.md`](../rt-dispatch-architecture.md)

## Historical decision (superseded)

Actions sharing one authored timestamp will have one immutable physical packet
view. The packet canonical order is every release before every activation. The
packet sender accepts validated instrument masks and builds one bounded
`SendInput` array of at most 30 events. This 30-event bound is historical; the
current production bound is 15 events. A partial activation or mixed result is
not interpreted as a known prefix.

The compiled authored schedule remains the source of metadata and generation
identity, while every physical dispatch uses the immutable packet view. There
is no production single-batch scan-code transport compatibility path.

The retry and late-timeline behavior recorded by the original implementation
milestones below is also historical. The current runtime performs no musical
retry after partial or zero-progress transport and never rebases the authored
timeline.

## Current normative contract

- A physical boundary is one bounded packet (at most 15 events); mixed packets
  keep canonical Up-before-Down order. Partial or zero-progress transport fails
  closed, with no musical retry, timeline rebase, or overdue catch-up.
- `Auto` priority uses MMCSS Games / AVRT High, with thread `Highest` fallback.
- Current normal Down and effective hold policy is in
  [ADR-0017](ADR-0017-current-down-continuity-and-physical-min-hold.md);
  lifecycle and final lease policy is in
  [ADR-0018](ADR-0018-resumable-release-barrier-and-final-lease-age.md) and
  [`rt-dispatch-architecture.md`](../rt-dispatch-architecture.md).

## Implementation status

The accepted PR-0–PR-9 history is retained as milestones, not as a second
runtime specification: PR-0 recorded repository/security gates (no perf
baseline artifact); PR-1 added backend snapshots and correctness-field checks,
without inferring hooks, Filter Keys, or game-side causes from latency;
PR-2 added immutable `CompiledPacket`/`PacketView`, timestamp grouping,
stale-Up suppression, canonical Up-before-Down masks, and duplicate-action
rejection; PR-3's whole-packet retry is superseded by
fail-closed transport; PR-4 moved same-timestamp batches to one transaction,
removed compatibility transport, and fault-tested partial mixed-packet cleanup;
PR-5 validated `game_fps` (its old timeline rebase is superseded); PR-6's old
priority order is superseded by the current order above, with estimator/spin
changes gated on Windows evidence; PR-7 added the pinned two-slot lock-free
snapshot buffer. PR-8/PR-9 were not implemented. The immutable source preserves
the milestone detail: [ADR-0001 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0001-packetized-native-input-dispatch.md).

## Security boundary

This ADR does not authorize game tampering, memory access, debugger or process
injection, anti-cheat bypass, keyboard hooks, or any input mechanism other than
Windows `SendInput`. The proposed keyboard-hook observer is rejected by the
P0 security mandate and is not part of acceptance evidence.

## Evidence boundary

`SendInput` return timestamps are sender-side evidence only. They do not prove
game polling, frame registration, rendering, or audio onset.

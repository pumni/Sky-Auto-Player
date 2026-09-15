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

- Physical packets are bounded to at most 15 events.
- Partial and zero-progress transport is fail-closed; there is no musical retry.
- Authored timestamps and generation identity remain immutable; there is no
  authored timeline rebase or overdue catch-up.
- In `Auto` priority mode, the production order is MMCSS Games / AVRT High,
  with thread `Highest` as the fallback.
- Timing feasibility, physical hold/release floors, authorization, and sender
  cutoff semantics are defined by
  [ADR-0010](ADR-0010-physical-timing-feasibility.md) and
  [`docs/rt-dispatch-architecture.md`](../rt-dispatch-architecture.md).

## Implementation status

The PR history below preserves the original implementation milestones. Any
clause explicitly marked superseded is not a description of current behavior.

- PR-0: repository gates and security baseline run; no perf baseline artifact
  was written because the repository instructions require explicit approval for
  immutable baseline artifacts.
- PR-1: live snapshot carries backend counters; Python rejects missing
  correctness-critical fields; latency warnings do not infer hooks, Filter
  Keys, or game-side causes.
- PR-2: `CompiledPacket` and zero-copy `PacketView` added; compiler groups by
  timestamp, canonicalizes Up before Down, suppresses stale Up masks, and
  rejects duplicate Up/multiple Down actions.
- PR-3 (historical, superseded): Win32 `PhysicalPacket` and bounded transaction
  outcome were added; the old milestone described bounded whole-packet retry
  for zero progress and Up-only recovery. Current production has no musical
  retry after partial or zero-progress transport.
- PR-4: multi-batch same-timestamp packets use one worker sender transaction;
  full success commits the packet once, while partial/zero activation fails
  closed through the existing cleanup path. Single-batch compatibility was
  removed after the packet-only transport migration. Mixed-packet partial fault injection verifies
  that the retrigger is not committed and uncertain physical state is cleaned.
- PR-5 (historical, superseded): resolved `game_fps` was validated at the
  native boundary; the old milestone described rebasing late timelines. The
  current authored timeline is immutable and does not rebase.
- PR-6 (historical, superseded): the old MMCSS acquisition order was Games,
  Low Latency, Audio. Current production `Auto` priority is MMCSS Games / AVRT
  High, with thread `Highest` fallback. Estimator and spin changes remain
  gated on Windows before/after evidence.
- PR-7: live worker metrics now publish through a two-slot lock-free snapshot
  buffer with reader pinning and coherence validation; the worker no longer
  takes a telemetry mutex for healthy publication.
- PR-8 and PR-9: not implemented in this checkpoint. Low-level keyboard-hook
  observation remains explicitly prohibited by the security boundary.

## Security boundary

This ADR does not authorize game tampering, memory access, debugger or process
injection, anti-cheat bypass, keyboard hooks, or any input mechanism other than
Windows `SendInput`. The proposed keyboard-hook observer is rejected by the
P0 security mandate and is not part of acceptance evidence.

## Evidence boundary

`SendInput` return timestamps are sender-side evidence only. They do not prove
game polling, frame registration, rendering, or audio onset.

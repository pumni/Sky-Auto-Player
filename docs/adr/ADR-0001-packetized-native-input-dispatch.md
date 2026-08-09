# ADR-0001: Packetized Native Input Dispatch

Status: accepted and implemented

## Decision

Actions sharing one authored timestamp will have one immutable physical packet
view. The packet canonical order is every release before every activation. The
packet sender accepts validated instrument masks and builds one bounded
`SendInput` array of at most 30 events. A partial activation or mixed result is
not interpreted as a known prefix.

The compiled authored schedule remains the source of metadata and generation
identity, while every physical dispatch uses the immutable packet view. There
is no production single-batch scan-code transport compatibility path.

## Implementation status

- PR-0: repository gates and security baseline run; no perf baseline artifact
  was written because the repository instructions require explicit approval for
  immutable baseline artifacts.
- PR-1: live snapshot carries backend counters; Python rejects missing
  correctness-critical fields; latency warnings do not infer hooks, Filter
  Keys, or game-side causes.
- PR-2: `CompiledPacket` and zero-copy `PacketView` added; compiler groups by
  timestamp, canonicalizes Up before Down, suppresses stale Up masks, and
  rejects duplicate Up/multiple Down actions.
- PR-3: Win32 `PhysicalPacket` and bounded transaction outcome added; full
  mixed packets use one `SendInput` call, with bounded whole-packet retry only
  for zero progress and Up-only recovery.
- PR-4: multi-batch same-timestamp packets use one worker sender transaction;
  full success commits the packet once, while partial/zero activation fails
  closed through the existing cleanup path. Single-batch compatibility was
  removed after the packet-only transport migration. Mixed-packet partial fault injection verifies
  that the retrigger is not committed and uncertain physical state is cleaned.
- PR-5: resolved `game_fps` is validated at the native boundary; the worker
  applies a frame-safe physical hold floor and rebases late timelines without
  frame-grid snapping or overdue catch-up bursts.
- PR-6: MMCSS Auto/Mmcss acquisition order is Games, Low Latency, Audio.
  Estimator and spin changes remain gated on Windows before/after evidence.
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

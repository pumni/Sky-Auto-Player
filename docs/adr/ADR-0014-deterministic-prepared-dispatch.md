# ADR-0014: Deterministic Prepared Dispatch

Status: Accepted.

Date: 2026-09-17.

Supersedes the normal-playback policy portions of ADR-0011, ADR-0012, and
ADR-0013. Those documents remain historical qualification records; they are
not the current normal-dispatch contract.

## Decision

Normal playback is a prepared absolute-frame dispatcher. The healthy precision
path is:

```text
immutable prepared physical frame
  -> absolute target = playback epoch + immutable offset
  -> HybridWaiter and bounded calibrated spin
  -> final cheap command/control, target, and focus atomics
  -> final_policy QPC evidence
  -> authoritative pre-call QPC
  -> exactly one prepared SendInput transaction
  -> completion QPC
  -> bounded post-send state, commit, and telemetry
```

The prepared frame and its authored deadline are immutable. Normal lateness is
observation, not suppression: a clean, authorized prepared frame that is due
or late gets one send attempt. Normal scheduling never retries a Down, rebases
the timeline, or treats completion latency as a later target's deadline.

Completion QPC is transport and telemetry evidence only in normal mode. It does
not feed a scheduling guard or move a later authored target. Strict/diagnostic
mode may retain `PhysicalTimingGuard` and its physical latest-start/floor
checks for qualification and diagnostics.

Focus and window identity are control-plane state plus a cheap atomic final
proof. The supervisor's published focus state is intentionally sampled rather
than synchronously querying Windows inside the target-crossing-to-SendInput
envelope. If Windows changes foreground before the supervisor publishes that
transition, one bounded observer race is permitted; a published focus loss
before the final atomic read suppresses the send.

The supervisor lease is a control-plane fail-safe. Its watchdog interrupts and
hard-stops the worker when the heartbeat expires; it is not a musical deadline
and cannot shorten a wait to an authored target.

Same-key geometry is validated before realtime execution. Invalid same-timestamp
Up/Down overlap is rejected at compile time, and positive but infeasible
release gaps are rejected at session admission. Every successful physical
packet has disjoint Up/Down masks. One physical boundary is one atomic
`SendInput` transaction.

Transport anomalies, including zero progress, partial progress, and integrity
loss, fail closed. They never become scheduler-lateness success and never
cause a normal Down retry. Cleanup and release diagnostics remain observable.

## Evidence retained

The dispatcher retains authored and physical target QPC, waiter wake QPC,
`final_policy_qpc`, authoritative pre-call QPC, completion QPC, interval
percentiles, transport status/counts, control/focus/lease rejection evidence,
preparation/no-allocation evidence, and bounded cleanup/hold/release forensics.
These measurements qualify the implementation; they are not additional
scheduling policy.

The legacy persisted key `default_normal_down_start_tolerance_us` is accepted
only for safe migration and is discarded from canonical settings. It is not
mapped to Timing Margin or any replacement grace period.

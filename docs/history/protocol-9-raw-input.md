# Historical Protocol-9 Raw Input Observer (Non-normative)

> [!NOTE]
> This document preserves the retired protocol-9 Raw Input observer mechanics and historical
> forensic equations. It is not an active production qualification contract and must not be
> used to interpret or migrate active calibration caches.
> For current timing and calibration contracts, see [timing-principles.md](../timing-principles.md),
> [hold-frame-model.md](../hold-frame-model.md), and [rt-dispatch-architecture.md](../rt-dispatch-architecture.md).

## Historical protocol-9 Raw Input observer mechanics

This records the retired observer protocol for forensics only. It is not a
production qualification contract and must not be used to interpret or migrate a
protocol-9 cache.

Calibration was a separate native, app-owned Raw Input proxy. It measured a
target-to-receipt total hold proxy, not game polling, rendering, audio, or
network latency. The sender boundary was measured as:

```text
validate/build/tag packet -> arm sequence -> prepare fixed INPUT array
-> wait to `T - 700 µs` with zero waiter spin
-> fused sender target_crossing/pre_call_qpc -> SendInput -> sendinput_completion_qpc
-> validate receipt
```

The calibration precision boundary followed production dispatch: tagged INPUT
materialization was complete before the handoff, the wait layer owned only the
low-occupancy wait to `T - 700 µs`, and the shared fused sender owned the final
QPC crossing, authoritative `P`, and the single `SendInput` call. No INPUT
construction or allocation was permitted after the handoff.

The Raw Input QPC timestamp was taken at entry to the `WM_INPUT` handler. The
same handler preserved the raw uint32-millisecond `GetMessageTime()` queue
timestamp as diagnostic evidence. Queue-time differences used modular uint32
subtraction across wraparound; queue time was not converted to the QPC epoch and
did not participate in qualification. Down and Up packets had to match
their direction, exact scan code, and extended flags. Publishable calibration
required the injection sequence tag to decode
and match on every receipt; Windows does not document preservation of
`KEYBDINPUT.dwExtraInfo` in `RAWKEYBOARD.ExtraInformation`, so a missing tag was
terminal rather than silently correlated. The pump-thread barrier handler
explicitly removed already-queued `WM_INPUT`
messages with an input-range filter before the next active packet was armed, so
missing tags could not silently alias stale receipts. If a packet was incomplete or
timed out, the correlation boundary was lost and the session could not arm another
packet; finding stale-generation evidence during the drain or normal dispatch
likewise prevented further packet arming. An active receipt with an incompatible
identity or direction, a duplicate, or a pending-receipt overflow was likewise
boundary-losing evidence; a scheduling class mismatch remained a rejected
sample and the only retryable measurement rejection. Parser/read failures,
reordered receipts, and chronology violations terminated the observer/session;
they were never converted into bounded retry noise. A clean
pair computed signed per-key values:

```text
D = down_receipt_qpc - down_completion_qpc
U = up_receipt_qpc - up_completion_qpc
T = target_qpc
P = pre_call_qpc
C = sendinput_completion_qpc
R = first_receipt_qpc

scheduler_shrink = (P_D - T_D) - (P_U - T_U)
sendinput_shrink = (C_D - P_D) - (C_U - P_U)
delivery_shrink = (R_D - C_D) - (R_U - C_U)
total_proxy_shrink = (T_U - T_D) - (R_U - R_D)

total_proxy_shrink = scheduler_shrink + sendinput_shrink + delivery_shrink
```

The five-key startup correlation probe verified physical All-Up read-only after
its balanced Down/Up sequence; it could not send an untagged All-Up packet while
the exact-tag observer was active. A publishable bucket performed one final
pump-thread-owned queue/trust seal: the pump entered `Sealing`, drained pending
`WM_INPUT`, restored its Raw Input registration, drained once more, and reached
`Sealed` before posting its exit and allowing physical cleanup. A failure before
or during that seal terminated the bucket and could not be published.

`R - C` was intentionally signed. The pump thread could observe a foreground
`WM_INPUT` while the measurement thread was still inside `SendInput`, so
`R < C` was valid and was not converted into a pairing anomaly. The
collector closed only after every expected `(sequence, scan code, direction)`
identity arrived; duplicates and unexpected receipts remained bounded
diagnostic evidence rather than satisfying completion early.

Each balanced pair anchored its Down target to the preceding packet's exact
`SendInput` completion plus the requested class gap. After that Down completed,
the Up target was derived from that exact Down completion plus the same gap.
Classification used the observed completion-to-entry idle interval, not the
requested target spacing. This kept a late or long Down syscall from silently
turning a requested cold Up into a hot sample; the pair was either observed in
the requested class or rejected with its diagnostic counters.

The publishable matrix had six buckets (`1/5/15 × hot/cold`) with at least
100 clean pairs in each. The configured `100` was a clean-pair target, not an
attempt cap: the native runner could make bounded additional attempts and
serialized the actual attempt count. If the target was not reached, the result
remained diagnostic/non-publishable and no empty quantile was accepted as
calibration evidence. Qualification used:

```text
candidate = positive global p99 + 100 µs

candidate <= 2,000 µs
    => calibrated, applied margin = max(300 µs, candidate)
candidate > 2,000 µs
    => out of the trusted correction envelope, no calibrated correction
```

An out-of-envelope measurement was complete evidence, was written as unhealthy
cache v5, and playback retained the user's selected Timing Margin (`500 µs`
for a fresh default configuration). A measurement/integrity failure preserved
the previous cache. The correction was
applied only to the hold floor; it never led the playback target, changed
`down_late_grace`, or claimed game-observed timing. Full-calibration checkpoints used plain-text SHA256
sidecars and a stable common provenance manifest. Resume and finalization
rejected any bucket whose source/build identity, native source fingerprint,
Rust version, QPC frequency, Windows build, CPU identity, topology,
efficiency-class histogram, or scheduling-aid acquisition labels differed from
the other five; only the observation timestamp could differ. Each native result
recorded the acquired MMCSS label, PowerThrottling/HighQoS guard state, and
waiter mode. Runtime cache loading also required the stable host fingerprint to
match the current native host. The final artifact and cache were published only
after the complete matrix and cache passed validation.

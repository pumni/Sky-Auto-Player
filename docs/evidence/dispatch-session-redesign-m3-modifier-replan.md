# M3 modifier-replan correction evidence

## Scope and source behavior

This correction addresses the P1 finding reviewed at `e8367c48ca8ef481d6946cd9058114d6c91ff1e9`. The replacement Final authorization was never invoked. The earlier runner-only attempt ended before any case (`run_count=0`); the later ReceiveOnly probes were no-input readiness checks. This correction has no native execution authorization.

Before the fix, final Down admission collapsed `ModifierHeld` into generic `Replan`. If a due Down remained eligible and no independent Up-only fallback was staged, `dispatch_staged_candidate` returned `Replan`; the worker loop selected `ContinueImmediately` and re-entered admission without waiting. The offline production-worker regression measured **39,786 modifier admission iterations in 120 ms** at the pre-fix source, with the mock sender receiving no Down. At corrected source, the same regression measured **74 iterations in 120 ms**; host timer scheduling can vary the exact count, while the test enforces a maximum of 256 and confirms no Down during the hold.

The corrected path preserves a distinct `ModifierHeld` result from final admission through staged dispatch to the worker loop. If a mixed Up+Down packet has a due independent Up, the existing Up-only fallback runs first. Only when no Up fallback is available does the worker wait to a checked QPC deadline 1,000 us after the current sample, using the existing `HybridWaiter`, the existing interrupt event, and zero spin threshold. Deadline or interrupt returns to the regular loop, which rechecks controls and immutable target validity before scheduler/admission work. Stop/quit, skip, panic, pause, and owner invalidation therefore interrupt or bound this wait; owner invalidation remains terminal. A released modifier only permits Down after a new final admission, including current foreground and retained owner checks.

This changes no musical D/E/W/S/C timestamps, H/G, QPC conversion, Frozen waiter policy, normal Due-path spin, MMCSS policy, one-attempt sender behavior, wire schema, or origin-only release/quarantine rule. The modifier backoff is not published as a musical wait observation.

## Offline worker-loop regressions

`engine::tests::modifier_replan` runs the actual `NativeDispatchSession` worker loop with a mock modifier query and `BackendConfig::Mock`. Its mixed packet contains an independent due Up plus a two-slot chord Down. It checks the Up-only packet is sent while Ctrl is held, the held interval has bounded admission iterations, pause/resume and focus loss interrupt the wait, and the released chord passes a fresh focus/owner/modifier gate. It asserts the exact packet sequence `Down(A), Up(A), Down(B+C), Up(B+C)`: no duplicate Down, split chord, or lost Up. Separate tests invalidate the owner during modifier backoff and exercise quit/Stop, skip, and panic; they assert no held-modifier Down, terminal owner loss, retained origin obligation with no cross-owner cleanup attempt, and bounded completion.

All these tests use mock senders. They do not invoke Windows `SendInput`, the native acceptance binary, or a ReceiveOnly qualification run.

## P2 owner-proof clone and lifecycle review

The existing test-only `ReleaseOwnerProof` clone counter was measured on prepared mock packet sends for fresh 1-, 5-, and 15-slot chords:

| Newly obligated Down slots | `ReleaseOwnerProof` Arc clone count | Count source |
|---:|---:|---|
| 1 | 2 | 1 outer proof snapshot + 1 per-slot journal proof |
| 5 | 6 | 1 outer proof snapshot + 5 per-slot journal proofs |
| 15 | 16 | 1 outer proof snapshot + 15 per-slot journal proofs |

The outer proof snapshot comes from `SessionTarget::try_release_owner_proof_for_generation`. Before the sender boundary, `TrackedKeyState::remember_release_owner_for_down` stores a proof for each newly obligated slot. `ReleaseOwnerProof::clone` increments the shared `Arc<ReleaseOwnerIdentity>` reference count; its retained `Arc<WindowIdentityAuthority>` and process object are shared and are not deep-cloned per slot. The test-only clone counter counts these proof clones, not process-handle duplication or elapsed cost. No optimization is proposed from counts alone.

The per-slot ledger preserves safety distinctions that one session-level proof would need to reproduce: missing and conflicted slot proofs fail closed; a mask spanning different origins is rejected; `active_mask`, `possibly_active_mask`, `in_flight_mask`, and `release_obligation_mask` track different send outcomes. The Down proof is journaled before the one-attempt sender call, so zero, partial, or uncertain transport cannot lose its origin. Musical and safety Up gather the slot proofs and require one matching owner. Safety cleanup additionally verifies the same target generation, PID, retained authority identity and continuous process proof; it never borrows a later foreground owner. Owner invalidation leaves unresolved release obligation/quarantine intact and suppresses an unverifiable Up.

Replacing these per-slot entries with a session proof is deferred until a design proves equivalent behavior for conflict, missing proof, uncertain/in-flight/possibly-active state, and origin-only release.

## P2 timing instrumentation review

No timing-path reorder was made. `observe_precision_packet_result` still records raw authored target, eligibility boundary, pre-call, completion, and prior completion evidence before scheduler and transport journal commit. Whether that work can be deferred is a paired QPC-provenance measurement question: compare `E−D`, `S−E`, `S−W` when a real W exists, and `C−S`, with observer on/off and cleanup evidence. No metric publication or commit ordering was changed based on source inspection alone.

## P3

No P3 cleanup was included in this correction.

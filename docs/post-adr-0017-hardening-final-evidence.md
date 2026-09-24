# Post-ADR-0017 Hardening Qualification — P8/H4

Status: final qualification evidence checkpoint; coordinator review pending.
The evidence below was collected from the fresh merged `origin/main` tree at
`70bcd7174cbcb6a286cde42d7ae38dae0fff7419`. P8 made no production behavior
change. This report does not close #376, #418, or umbrella #371.

## Revisions, host, and accepted phase history

The P8 baseline immediately after #426 was `8231e381492852d88bbd9a8bd3922f00084c905a`.
H4a PR #428 fixed the accepted stale power-callback defect in #427, with base
`8231e381492852d88bbd9a8bd3922f00084c905a`, PR head
`993f3febbe89d4d7c078360d96010a5991cd4657`, and merge commit
`70bcd7174cbcb6a286cde42d7ae38dae0fff7419`. A fresh fetch confirmed both
`origin/main` and the qualification checkout at that merge SHA. All final
native case manifests record that source revision and `source_tree_clean=true`.

Host toolchain: Rust/Cargo 1.98.1, `x86_64-pc-windows-msvc`, Windows 11.
Post-merge CI for the exact qualification source SHA passed:
[workflow run 35919423092](https://github.com/pumni/Sky-Auto-Player/actions/runs/35919423092).

| Phase | Finding / disposition | Base SHA | PR head | Accepted merge SHA |
|---|---|---|---|---|
| P0 | Current-main rebaseline; accepted on `e5bf883fd8549005d1bc02bc98f4e14d1dc16e0b`; no PR | `e5bf883fd8549005d1bc02bc98f4e14d1dc16e0b` | — | `e5bf883fd8549005d1bc02bc98f4e14d1dc16e0b` |
| P1 / #372 | Lease tied to actual monitor progress; #372 completed | `e5bf883fd8549005d1bc02bc98f4e14d1dc16e0b` | #419 `a3c7a13205d00e23a610eab0606e02d022cb5ade` | `5b726e177eeeb4717f14b4bc9ffefe79a74608d9` |
| P2 / #377 | Old orphan authored-Up defect not reproduced on the shipping prepared path; disposition remains `not_planned` | `5b726e177eeeb4717f14b4bc9ffefe79a74608d9` | #420 `c14b6289a78563097b4d9aa988ec71c7d310f51c` | `4edf7751722512d326e8bec78ed9b375f2d13405` |
| P3 / #378 | Lifecycle release obligations scoped to tracked ownership | `4edf7751722512d326e8bec78ed9b375f2d13405` | #421 `59334d741cc906d933861afa0b8d89fc734f97e5` | `2896a889c1c19469bd2dbc7ca5bb036dfa03ebe8` |
| P4 | Healthy natural completion requires authoritative generation accounting | `2896a889c1c19469bd2dbc7ca5bb036dfa03ebe8` | #422 `48ae0bdbabbefbb8e7a48888fad7ea6deeb87635` | `ac153fc08e769ae0806506b0207bfc9a4db88fe2` |
| P5 / #373 | Target HWND/generation authority made ABA-safe | `ac153fc08e769ae0806506b0207bfc9a4db88fe2` | #423 `b9f9b02727636f5f208ae479dc0e8cfc4e9ede2c` | `11ec8b00831e2bc08d390cdad3c0180e82d8f36b` |
| P6 / #374 | Focused Down binds to startup window-owner identity | `11ec8b00831e2bc08d390cdad3c0180e82d8f36b` | #424 `3e51c48094c37d097f21e7b97c57e1358ac1240c` | `03da9d4df93e19f080336d6d8dd96420adc8fba6` |
| P7a / #375 | Physical-key interference and cost characterized; no note-key probe approved | `03da9d4df93e19f080336d6d8dd96420adc8fba6` | #425 `20b83335bf14db694b9623938f3c069c04a50a7d` | `2d14025e46963ac5ea25e173f8453138a3694c29` |
| P7b / #375 | Fixed five-modifier Down guard; #375 closed after merged evidence with zero-return, query-to-send, and same-note residuals retained | `2d14025e46963ac5ea25e173f8453138a3694c29` | #426 `c6b507ba0f89243b9ed0534d9bacc2c76b603bd4` | `8231e381492852d88bbd9a8bd3922f00084c905a` |
| H4a / #427 | Stale power callback epoch/rundown fence | `8231e381492852d88bbd9a8bd3922f00084c905a` | #428 `993f3febbe89d4d7c078360d96010a5991cd4657` | `70bcd7174cbcb6a286cde42d7ae38dae0fff7419` |

## Proposed issue dispositions after #429 merges

#376, #418, and umbrella #371 remain open at this checkpoint. The proposed
disposition is to close all three as `completed` after #429 merges, using the
merged-tree P8/H4 evidence and required CI as the completion evidence. This
report does not close them. Keep #377 as `not_planned`. Preserve the accepted
residuals listed below with any of those closures.

The P8 runtime matrix below uses only the final H4a merge tree. Earlier PR
checks and partial P8 runs are phase history or fix evidence, not final P8/H4
results.

## H4 system-power callback/reset qualification

The accepted pre-fix reproduction showed an old process-lifetime resume
callback clearing a newer session's suspend state after `reset_for_new_session()`.
The merged implementation gives callback mutation authority to one even
`callback_epoch`: a callback captures the epoch, increments the in-flight
count, and rechecks the epoch before it can mutate anything. Lifecycle reset
closes the epoch, waits for already-authorized callbacks to drain, clears state,
boundary, counters, and event state, then opens the next epoch. The
sequentially consistent close/acquire/recheck order makes either the callback
lease visible to rundown or the closed epoch visible to the callback. The
callback itself takes no mutex; its state-CAS loop is bounded to eight
attempts. Epoch exhaustion fails closed rather than wrapping.

Post-merge tests passed:

| Case | Result |
|---|---|
| `stale_resume_callback_crossing_session_reset_can_clear_new_session_suspend` — stale callback cannot clear the newer suspend state, counters, boundary, pending state, or signal | 1 passed |
| `stale_suspend_cannot_publish_boundary_counters_or_signal_after_reset` and the `stale_` callback suite | 21 stale tests passed |
| `reset_waits_for_callback_that_already_acquired_epoch_authority` — reset drains a callback that already holds authority | 1 passed |
| `system_suspend_after_wait_wake_blocks_final_down_admission_until_revalidated_resume` | 1 passed |
| `stale_epoch_resume_cannot_authorize_due_worker_down` — zero worker sender calls until current-epoch resume | 1 passed |
| Desktop `power_lifecycle::tests` callback registration/routing group | 9 passed |
| Full `sky_player` test-support library | 391 passed |

The targeted state and worker regressions were run with:

```powershell
rtk cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib stale_resume_callback_crossing_session_reset_can_clear_new_session_suspend -- --nocapture
rtk cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib stale_ -- --nocapture
rtk cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib reset_waits_for_callback_that_already_acquired_epoch_authority -- --nocapture
rtk cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib system_suspend_after_wait_wake_blocks_final_down_admission_until_revalidated_resume -- --nocapture
rtk cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib stale_epoch_resume_cannot_authorize_due_worker_down -- --nocapture
```

The first test's historical name describes the former failure mode; its current
assertions verify that the stale callback cannot clear the new suspend. The
system-power tests were rerun from the merged tree and are part of final H4
evidence.

## Final ReceiveOnly native matrix

The release harness was built from the merged tree and each case ran with a
500 µs Timing Margin against the project-owned ReceiveOnly sink. The suite
contains 29 passing runs, including one dense-alternating run under CPU
contention. All 29 manifests identify source SHA
`70bcd7174cbcb6a286cde42d7ae38dae0fff7419` and a clean source tree.

| Scenario | Evidence directory |
|---|---|
| canonical-single | `.benchmarks/physical-native-cases/native-case-20260924T041848-113787db` |
| canonical-chord | `.benchmarks/physical-native-cases/native-case-20260924T041853-f1ada304` |
| canonical-max-chord | `.benchmarks/physical-native-cases/native-case-20260924T041857-fffd0093` |
| hold | `.benchmarks/physical-native-cases/native-case-20260924T041901-38365ce8` |
| long-single-sequence | `.benchmarks/physical-native-cases/native-case-20260924T041905-92c1f83e` |
| dense-alternating, quiet | `.benchmarks/physical-native-cases/native-case-20260924T041920-f8e06be2` |
| chord-sweep | `.benchmarks/physical-native-cases/native-case-20260924T041924-d0d0fe92` |
| near-minimum-retrigger | `.benchmarks/physical-native-cases/native-case-20260924T041930-9132673b` |
| rapid-retrigger | `.benchmarks/physical-native-cases/native-case-20260924T041934-9aa776d1` |
| release-gap-stress | `.benchmarks/physical-native-cases/native-case-20260924T041938-d091b8a9` |
| mixed-up-down | `.benchmarks/physical-native-cases/native-case-20260924T042012-a193441a` |
| ambiguous-packet | `.benchmarks/physical-native-cases/native-case-20260924T042016-216196b9` |
| preflight-user-held | `.benchmarks/physical-native-cases/native-case-20260924T042020-a4bd58f1` |
| modifier-held-final-boundary | `.benchmarks/physical-native-cases/native-case-20260924T042024-9b11dad8` |
| modifier-held-after-owned | `.benchmarks/physical-native-cases/native-case-20260924T042028-4d2e3a00` |
| cleanup-full-release | `.benchmarks/physical-native-cases/native-case-20260924T042032-4dd51a87` |
| focus-loss | `.benchmarks/physical-native-cases/native-case-20260924T042043-c8043a5c` |
| target-hwnd-change | `.benchmarks/physical-native-cases/native-case-20260924T042053-f404eb18` |
| owner-mismatch | `.benchmarks/physical-native-cases/native-case-20260924T042057-25855793` |
| owner-query-failure | `.benchmarks/physical-native-cases/native-case-20260924T042101-058d0ee7` |
| owner-process-termination | `.benchmarks/physical-native-cases/native-case-20260924T042106-455e0114` |
| pause-resume | `.benchmarks/physical-native-cases/native-case-20260924T042119-ac22bed8` |
| suspend-resume | `.benchmarks/physical-native-cases/native-case-20260924T042123-8f7fd4f1` |
| stop-cleanup | `.benchmarks/physical-native-cases/native-case-20260924T042128-7f11649c` |
| skip-cleanup | `.benchmarks/physical-native-cases/native-case-20260924T042132-a47a8dd7` |
| supervisor-lease-expiry | `.benchmarks/physical-native-cases/native-case-20260924T042146-4cced265` |
| w4-noncanonical | `.benchmarks/physical-native-cases/native-case-20260924T042153-156b472d` |
| timing-margin-sweep | `.benchmarks/physical-native-cases/native-case-20260924T042157-e1bada77` |
| dense-alternating, CPU contention | `.benchmarks/physical-native-cases/native-case-20260924T042205-cec4aac0` |

Across these 29 runs: 1,492 sink events, zero stuck keys, and zero failed
releases. The `suspend-resume` run observed exactly one suspend and one resume;
it has one production-forensics anomaly classified as an unmatched Up, with
zero stuck keys and zero failed releases. This is a safety-Up forensic residual
accepted for carry-forward: lifecycle cleanup may emit a safety Up before a
later authored Up, so raw physical Up count need not equal musical generation
release count.

Normal-mode timing evidence across the matrix:

| Measurement | Samples | Minimum observed | Required floor | Violations | Affected scenarios |
|---|---:|---:|---:|---:|---|
| Up pre-call after prior successful Down completion | 724 | 171,697 QPC ticks (17,169.7 µs) | 171,670 ticks (17,167 µs) | 0 | None |
| Next same-key Down pre-call after prior successful Up completion | 673 | 166,796 QPC ticks (16,679.6 µs) | 166,670 ticks (16,667 µs) | 0 | None |

The floors are those materialized for the 500 µs margin at 10 MHz QPC: the
first is the effective minimum hold, the second is one frame. These are
sender-side ReceiveOnly measurements; they do not prove game sampling.

## Deterministic hardening matrix and global verification

The final merged-tree tests and ReceiveOnly cases cover:

| Risk area | Final evidence / disposition |
|---|---|
| Supervisor stall, unrelated helper alive, late progress, lease equality, and post-expiry Down | P1 deterministic lease tests plus final `supervisor-lease-expiry` ReceiveOnly run; terminal lease remains latched and blocks new Down. |
| Target A→B→A ABA, rapid target transitions, and stale owner binding | `target_aba_rejects_stale_owner_authority_until_full_rebind`; `target-hwnd-change` native case. |
| Numeric HWND reuse, owner query failure, owner process exit | P6 deterministic tests plus `owner-mismatch`, `owner-query-failure`, and `owner-process-termination` native cases. |
| Focus loss/restore and published-focus races | `final_admission_requires_fresh_foreground_match_and_rechecks_atomic_focus`, focus lifecycle tests, and `focus-loss` native case. |
| Startup held key, modifier held on first Down/after existing ownership, and UpOnly behavior | P7a deterministic key-state seam; `preflight-user-held`, `modifier-held-final-boundary`, and `modifier-held-after-owned` native cases. The guard queries five modifier VKs on Down-bearing packets and none on UpOnly. |
| Zero/partial/ambiguous transport, post-send clock uncertainty, QPC failures, and cleanup ownership | Win32/player deterministic test-support suites; `ambiguous-packet`, `mixed-up-down`, and `cleanup-full-release` native cases. No musical Down replay; release obligation is `active_mask | possibly_active_mask | failed_release_mask | in_flight_mask`. |
| Pause, suspend/resume, stop, skip, and terminal cleanup | Worker lifecycle regressions and the corresponding final native cases above, including the stale callback reset and due-Down regressions. |
| Maximum chord, dense alternating, retrigger, release gap, and timing margin | `canonical-max-chord`, `dense-alternating`, `near-minimum-retrigger`, `rapid-retrigger`, `release-gap-stress`, and `timing-margin-sweep`; quiet plus CPU-contention dense case. |

Post-merge verification passed:

| Check | Result |
|---|---|
| `cargo xtask check static` | PASS |
| `cargo xtask check rust` | PASS |
| `cargo xtask check all` | PASS |
| `cargo test -p sky_dispatch_core` | 73 passed |
| `cargo test -p sky_dispatch_win32 --features test-support` | 248 passed, 1 ignored |
| `sky_player` test-support library | 391 passed |
| `sky_player` test-support `rt_dispatch_no_alloc` | 23 passed |
| Desktop `power_lifecycle::tests` | 9 passed |
| Prepared acceptance | `acceptance_clean=true`; 7 full physical boundaries, 7 full sends, zero anomalies |
| `git diff --check` on merged runtime tree | PASS |

The final native release harness was built with
`cargo build --locked --manifest-path rust/Cargo.toml -p sky_player --release --features real-input-acceptance,test-support --bin rt-native-acceptance`.
The project ReceiveOnly runner was used for the 29 scenarios. No destructive
Windows shortcuts were sent; cases involving physical user-key state used the
accepted deterministic seam.

## Optimized and no-allocation audits

On the merged production build:

- `audit_prepared_normal_assembly.ps1` passed: one sender transaction, zero
  retained precision-helper calls, and no scoped forbidden calls or copy/
  division symbols in the prepared normal sender suffix.
- `audit_modifier_guard_assembly.ps1` passed: five modifier queries in the
  expected order, one prepared dispatch probe, one sender transaction, and no
  allocation, mapping, foreground/process inspection, extra QPC, lock, or
  logging calls in the scoped healthy suffix beyond accepted sender-owned
  pre-call instrumentation.
- The `rt_dispatch_no_alloc` test passed (23/23).

The broader legacy `audit_dispatch_assembly.ps1` reported
`recover_missed_down_boundary: NOT_FOUND (required clean target)`. This is
accepted as a legacy tooling/symbol-retention limitation, not a runtime defect;
inlining into the optimized caller may explain the missing symbol. The scoped
`audit_prepared_normal_assembly.ps1` and
`audit_modifier_guard_assembly.ps1` results above are the acceptance evidence
for the shipping suffix. The legacy audit does **not** independently certify
the optimized body of the recovery helper, so it remains incomplete and is not
described as clean.

Seven real-wait-core gap benchmarks (5, 20, 25, 100, 250, 500, and 1000 ms)
also passed dispatch/waiter cleanliness checks with zero misses. They are
diagnostic-only: sample counts were intentionally small, every report has
`statistics_eligible=false`, transport was deterministic mock transport, and
these measurements do not qualify production sender latency. Exact reports:
`.benchmarks/p8-merge-70bcd717-real-wait-core-5ms.json`,
`20ms.json`, `25ms.json`, `100ms.json`, `250ms.json`, `500ms.json`, and
`1000ms.json` in the same directory.

## Current source-of-truth crosswalk

| Contract | Current source / result |
|---|---|
| Normal current prepared Down makes one attempt only after live lifecycle, target, focus/owner, modifier, suspend, lease, and preflight gates | `sky_player::engine::worker::dispatch::prepared::send_prepared_normal_precision_frame`; `worker/admission.rs`; `sky_dispatch_win32::input::tracked::packet_send`; ADR-0017. Fresh foreground and owner proof occur for eligible focused Down. |
| Mixed/chord packet is one Up-before-Down `SendInput` transaction | `sky_dispatch_win32::input::packet` packet construction and one-call sender; ReceiveOnly `canonical-chord`, `canonical-max-chord`, and `mixed-up-down` pass. |
| Failed, partial, ambiguous, or clock-uncertain musical Down is never replayed | Prepared dispatch terminates the attempt; tracked transport preserves the conservative `active_mask | possibly_active_mask | failed_release_mask | in_flight_mask` release obligation. |
| Physical hold/repress policy | `PhysicalTimingGuard::observe_successful_packet`: successful Down completion + materialized effective minimum hold; successful Up completion + one frame for next same-key Down. 724/673 native pairs had zero floor violations. |
| Target authority | P5 coherent HWND/generation publication rejects ABA; P6 binds the target to startup owner PID/process identity and rechecks current foreground HWND/owner PID at final focused Down admission. |
| Supervisor lease | P1 progress comes from the actual monitor; a shared atomic deadline/latch rejects late progress and cannot be healed after expiry. |
| Physical-key preflight | Startup/preflight samples the 15 instrument-note keys and caches by target stamp. A zero `GetAsyncKeyState` result is `NoHeldObserved`, not proof of physical AllUp. Instrument-note keys are not sampled again on each Down. |
| Modifier guard | P7b samples `VK_LWIN`, `VK_RWIN`, `VK_CONTROL`, `VK_SHIFT`, and `VK_MENU` for Down-bearing packets. A detected held modifier rejects the Down; UpOnly remains independent. |
| Cleanup ownership | `release_obligation_mask = active_mask | possibly_active_mask | failed_release_mask | in_flight_mask`; zero ownership produces no cleanup keyboard event, and ambiguous packet cleanup is scoped to the uncertain packet ownership. |
| Healthy natural finish | `clean_completion_proven` requires total = activated = released, zero drop/cancel/anomaly buckets, and zero final release obligation. Lifecycle termination has distinct semantics. |

The documentation search found no current normative claim that the bounded
spin skips interrupt-generation polling, that final Down admission uses only
cached target/focus atomics, that zero preflight samples prove AllUp, that
preflight remains fresh for a whole session, that cleanup always sweeps all 15
keys, or that `released + dropped_expired` is healthy completion. The P7a
method now labels its cached note-key probe as the P7a baseline and distinguishes
the later P7b modifier guard. Historical ADRs were left unchanged.

## Win32 assumptions and limits

- [`SendInput`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
  reports how many events were inserted into the system input stream and inserts
  a batch serially without interspersing other input. It does not prove the game
  sampled or rendered those events. UIPI can block injection without an
  identifying return value or `GetLastError` result; the code treats incomplete
  sends as terminal and preserves cleanup ownership.
- [`GetAsyncKeyState`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate)
  exposes the high bit as current-down state, but zero is also a failure result.
  The low “pressed since last query” bit is unreliable. Therefore a zero sample
  cannot prove a physical key is up.
- [`GetForegroundWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getforegroundwindow)
  can return NULL during activation changes. [`GetWindowThreadProcessId`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid)
  returns the creating thread and optional process ID, and zero for an invalid
  HWND; these failures are treated as no Down authority.
- [`IsWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindow)
  explicitly warns that another thread's window can be destroyed after the
  check and that recycled HWND values may identify a different window. The
  retained process identity and final owner query close numeric reuse across
  different owners, but same-process window recreation between checks and the
  final owner-validation-to-`SendInput` race cannot be eliminated by these APIs.
- [`PowerRegisterSuspendResumeNotification`](https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-powerregistersuspendresumenotification)
  registers the callback recipient, and
  [`PowerUnregisterSuspendResumeNotification`](https://learn.microsoft.com/en-us/windows/win32/api/powerbase/nf-powerbase-powerunregistersuspendresumenotification)
  cancels that registration. The design does not rely on unregister providing
  rundown for callbacks already in flight; its own epoch/rundown gate protects
  per-session state.

## Residual risks and qualification disposition

Risk summary for the coordinator:

| Risk | Final disposition |
|---|---|
| Wrong musical note Down | A same-note user key held after cached preflight can still conflict with an injected note; this accepted P7 residual remains. |
| Stale Down after a prior-session resume callback | No post-merge reproduction; epoch/rundown state tests and the worker zero-sender-call regression pass. This is regression evidence, not a formal proof of all Windows schedules. |
| Wrong-window Down | Observed owner mismatches/query failures are rejected. Same-process HWND recreation between checks and the owner-check-to-send race remain bounded OS-level risks. |
| Shortcut-modified Down | A held modifier reported by the high bit is rejected. Modifier state can change between its final query and `SendInput`; a zero return can also hide query failure and therefore a held modifier. |
| Stuck-key ambiguity | Partial/uncertain transport retains ownership for bounded cleanup; all 29 sink runs ended with zero stuck keys and failed releases. OS blocking or failed cleanup cannot be ruled out for every receiver state. |

| Residual | Disposition / impact |
|---|---|
| A user presses an instrument-note key after cached preflight | P7a reproduced the stale note-key observation. P7b deliberately did not add per-note sampling: `GetAsyncKeyState` cannot distinguish the user's held note from a note already owned/injected by this session, especially on same-key retrigger and Mixed Up+Down. A conflicting physical note Down remains possible and is an accepted program residual. |
| Zero from `GetAsyncKeyState` can mean key-up or query failure | The instrument preflight reports `NoHeldObserved`, not verified AllUp. A failure that returns zero can also hide a held modifier from the modifier guard; no reliable error channel exists to fail closed only on query failure. |
| Modifier query-to-send race | A modifier can change after the fixed five queries and before the single `SendInput` call. No bounded final check can eliminate that interval. |
| Window can be destroyed/recreated by the same process between supervisor checks; foreground/owner validation can race with the following send | The current PID/process identity and fresh owner check reject observed owner changes, but Windows exposes no stable HWND lifetime token here and validation-to-send cannot be atomic. No such wrong-window send was reproduced in the final matrix. |
| SendInput success is system-stream insertion, not game receipt/audio | The ReceiveOnly sink proves the project's sender-side event stream only. Game-side sampling, rendering, and sound onset remain unmeasured. |
| Suspend/resume forensic unmatched Up | One in the final `suspend-resume` run; accepted safety-Up accounting residual, with zero stuck keys and failed releases. |
| Legacy optimized symbol audit limitation | `recover_missed_down_boundary: NOT_FOUND` is accepted as a legacy tooling/symbol-retention limitation, not a runtime defect. The scoped `audit_prepared_normal_assembly.ps1` and `audit_modifier_guard_assembly.ps1` results are the shipping suffix acceptance evidence; the legacy audit does not independently certify the optimized recovery-helper body and remains incomplete, not clean. |
| Small real-wait-core benchmark samples | Diagnostic evidence only; not statistical qualification or production sender-latency evidence. |

No new defect was deterministically reproduced after #428 merged. The original
stale callback defect has post-merge state, desktop routing, and worker
Down-authority regressions. This checkpoint preserves the accepted P7 physical
note/zero-return residuals and the optimized-audit limitation for coordinator
disposition; it does not claim bug-free, race-free, formally verified, or
game-receipt-proven behavior.

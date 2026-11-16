# Issue #666 Preflight and Code-Quality Correction

## Revision and scope

- Reviewed baseline: `4248d5c90b62cf5b24ab3802fa06bdda04b47fb2`
- Baseline tree: `a138d2d28c84083fbc4e6081c8d82c9b7074d9f4`
- Baseline parent: `a1ec8f64`
- Candidate implementation commit: `7bbdb3f0af498f88009a04afc45692d817c783bf`
- Candidate implementation tree: `08515d11497e88633ac07b3e6846735c716ba4cb`
- Candidate implementation parent: `4248d5c90b62cf5b24ab3802fa06bdda04b47fb2`
- Branch: `refactor/prepared-playback-runtime`
- PR: [#667](https://github.com/pumni/Sky-Auto-Player/pull/667), remains Draft.

This correction preserves an already validated target preflight across a transient
owner-proof or final-permit Busy. It does not cache a foreground, PID, process
identity, or release-owner proof. The accepted bounded Replan policy and the
previous musical Up, stale-poll, raw-zero, and nonblocking-permit corrections
remain in place.

## Preflight cache behavior

Before this correction, `apply_final_down_rejection()` cleared
`WorkerRuntime.verified_target` for every `OwnerRejected(_)`, including
`OwnerRejected(ProofBusy)`. The final permit error branch also cleared the stamp
before separating `WriterBusy`, `OwnerAuthorityBusy`, and `NotCurrent`. A due
Down restage then repeated the all-instrument-keys-up preflight. If A was
legitimately held by the session, the repeated scan could reject the healthy
hold or return inconclusive evidence.

Now both transient Busy paths call `retain_current_preflight_for_retry()`. The
helper preserves only an existing stamp equal to the candidate's exact
`TargetStamp`, and only while that HWND/generation remains published. It clears
an absent, stale, or mismatched stamp and never writes a new stamp. `NotCurrent`,
focus recovery, target drift, and real invalidation continue to clear/revalidate
through their existing paths. The retained stamp has no authority over the
fresh final checks: foreground, owner PID, process identity continuity, lease,
power, control, generation, and the final writer/owner permit are checked again
for every attempt.

## Deterministic worker regressions

`dispatch_loop_preflight_busy_tests.rs` drives the production worker staging,
final admission, sender boundary, and outer replan policy. It uses the existing
test-support preflight probe and barrier/channel synchronization; no sleep is
used to create a lock race. Its initial scan succeeds. After Down(A), the probe
reports A held, so any accidental scan on the Busy retry would be observable.

| Case | Preflight probes | During Busy / invalidation | After unlock or recovery |
|---|---:|---|---|
| Early fresh owner-proof read Busy | 1 before, 1 after Busy, 1 after retry | No B syscall; cursor stays 1; active/release A masks stay `0x0001`; possible/in-flight stay zero; A frozen proof is unchanged | B dispatches once; A's authored Up dispatches under the retained A proof |
| Final owner-authority mutex Busy | 1 before, 1 after Busy, 1 after retry | Barrier confirms the worker reached the final permit after fresh identity admission; no B syscall or state mutation | B dispatches once after release; A Up remains independently timed |
| Final writer mutex Busy | 1 before, 1 after Busy, 1 after retry | Same final-boundary assertions as owner-authority Busy | B dispatches once after release; A Up remains independently timed |
| Same-HWND generation ABA | 1 for old generation, 2 after valid new-generation staging | The numeric HWND is unchanged but the generation differs; old stamp is not reused | New generation is preflighted; A's frozen proof still names the old generation |
| Focus loss and recovery | 1 before loss, 2 after cleanup/recovery | Focus rejection clears the stamp; verified cleanup releases A | Normal focus-restoration preflight succeeds before a new Down can proceed |
| Lease expiry during Busy | 1 before, 1 after Busy | No B call; A remains active and obligated; lease expiry clears the one-shot retry | No contention retry is dispatched after expiry |

For each Busy case, the sender capture contains only the already completed
Down(A) before unlock. Its raw result is one attempt with `first_inserted=1`.
After retry the sequence is exactly Down(A), Down(B), Up(A); each has one
attempt and `first_inserted=1`. The cursor is 1 and masks are active=`0x0001`,
possible=`0`, in-flight=`0`, release=`0x0001` during Busy. After B commit the
cursor is 2 and active/release masks are `0x0003`; after authored Up(A), active
and release masks are `0x0002`. The successful A/B/Up(A) path has no terminal
error. No raw-zero or ambiguous sender result is replayed or reclassified by
this change.

Fresh retry checks are asserted through the harness counters and control seam:
owner-authority read, generation revalidation, final writer permit, foreground
query, owner PID query, process-continuity query, lease/control boundary, and
power/control state. The test-support synthetic process authority does not call
native `GetProcessTimes`; its expected count is zero. The lock tests reach final
permit contention only after the fresh process identity path completes.

## Production warning cleanup

The five warnings reported by the reviewed Tauri CI job are removed at their
source:

| Warning site | Change | Production evidence |
|---|---|---|
| `worker.rs`: `DispatchAttemptOutcome` re-export | Re-export is limited to test and `test-support` consumers | No-default `sky_player` Clippy and Tauri checks pass with `-D warnings` where applicable |
| `dispatch_loop.rs`: test-only `runtime` argument | The argument and call arguments are cfg-gated with the test hook | No-default production-like build and strict Clippy pass |
| `target.rs`: `bind_owner_identity` | Helper is limited to tests/test-support; production session binds retained authority through `bind_owner_identity_authority` | Both feature configurations compile cleanly |
| `dispatch/mod.rs`: `TerminateStatic` | Removed the unconstructed variant and its match arms; owned terminal error strings retain the same exit semantics | All-target/all-feature strict Clippy and desktop-native checks pass |
| `DispatchAttemptOutcome::public_outcome` | Conversion is gated to the harness's test/test-support consumer | Test-support suite and no-default strict Clippy pass |

Strict no-default-features Clippy also identified the intentionally large inline
`DispatchObservation::Down` variant. It remains inline in the bounded observer
queue to preserve zero-allocation dispatch. A narrowly scoped
`expect(clippy::large_enum_variant)` documents that production layout contract;
no `unused`/`dead_code` suppression was added. The requested no-default strict
Clippy command and the all-target/all-feature strict Clippy command both pass.

## Bounded cleanup and compatibility

| Old path / helper | Status | Actual caller | Invariant retained | Evidence |
|---|---|---|---|---|
| `apply_final_down_rejection(OwnerRejected(_))` clears preflight | Consolidated | Musical final Down admission | Only an already validated same-generation preflight survives transient Busy | Six `preflight_busy` worker tests |
| Final permit clears stamp before classifying errors | Consolidated | Final writer/owner-authority permit | Busy is pre-syscall and leaves cursor/journal untouched; stale proof still replans fail-closed | Barrier tests for both mutexes |
| `OwnerPermitBusySource` one-field projection | Removed | Final Down admission caller needs only Busy vs Replan | Busy remains distinct from target drift/control replan | Strict Clippy and bounded outer-loop regressions |
| `take_permit_busy_retry()` wrapper | Removed | Production outer worker loop and tests | One-shot retry token still consumed with `mem::take` | Due contention outer-loop trace |
| `DispatchReplanAction::WaitCompleted` timing payload | Consolidated into a test-only thread-local trace | Production loop needs only completion vs Exit | Selected retry deadline and interruption remain observable in tests; production result has no timing payload | Bounded Replan test and explicit interrupt test |
| `WorkerRuntime.previous_sendinput_completion_qpc` and `previous_sender_completion_known` plus redundant stores | Removed | No legitimate readers existed | Observation/telemetry fields and local attribution values remain unchanged | Full player suite and schema-25 tests |
| Common milestone test include scope | Consolidated into `transport_outcomes`, `owner_proof`, and `proven_raw_zero` modules | `dispatch_loop` test module | Shared harness fixtures remain available; deterministic barriers are retained | Full player suite |
| `m4_guard_ab_benchmark` module identifier | Renamed to `guard_ab_benchmark`; benchmark file and runner contract retained | Offline benchmark test filter | File, runner, environment variables, and output markers are unchanged | The runner's filter lists exactly one ignored test |
| `DispatchStep::TerminateStatic` | Removed as unconstructed | Observer/admission terminal result matches | Owned terminal diagnostic and terminal exit behavior remain | Strict Clippy and desktop-native checks |

No public Win32 wrapper, polling API, DTO, telemetry schema, UI contract,
default timing value, spin policy, shared arena constructor, startup baseline,
watchdog/lease/power policy, or presentation failure policy was changed.
Remaining architecture cleanup listed as out of scope by the task remains
deferred. The A/B benchmark was listed but not run; it remains waiter-only and
does not qualify gameplay.

## H/G tick stability

The raw-zero H/G regression now supplies exact QPC values directly to scheduler
evaluation for floor minus one tick, exact floor, and plus one tick. This removes
real scheduling delay from arithmetic assertions. Dispatch still commits the
mock sender events at the chosen synthetic boundary. A separate real waiter
smoke test waits on a future 5 ms QPC deadline and verifies a `Due` wake; the
separate owner-permit interrupt regression verifies interrupt wake behavior.
No timing default or H/G policy changed.

## Verification ledger

Toolchain: Windows x64 MSVC (`x86_64-pc-windows-msvc`), `rustc 1.99.1`
(`58cf8f9c3`), `cargo 1.99.1` (`5f94df478`).

| Command | Result |
|---|---|
| Focused early proof Busy regression | Pass, 1 test |
| Focused `preflight_busy::` regressions | Pass, 6 tests |
| Focused owner-proof module | Pass, 11 tests before test-module path cleanup; the final full suite also covers the same cases |
| H/G exact-boundary regression | Pass, 1 test |
| H/G real waiter smoke | Pass, 1 test |
| Due Down outer Replan regression | Pass, 1 test; sender attempts=1, raw inserted=1, bounded wait selected at 1,000 µs |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | Pass, exit 0 |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | Pass, exit 0 |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_player --lib --no-default-features -- -D warnings` | Pass, exit 0, no emitted warnings |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | Pass, exit 0, no emitted warnings |
| `cargo build --locked --manifest-path rust/Cargo.toml -p sky_player --lib --no-default-features` | Pass, exit 0, no emitted Rust warnings |
| Tauri candidate `cargo check` for `sky_desktop_shell` binary with default production features and local `TAURI_CONFIG={"build":{"frontendDist":null}}` | Pass, exit 0, no emitted Rust warnings |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | Pass, 84 passed, 0 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | Pass, 278 passed, 1 ignored child-process helper |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | Pass, 317 passed, 1 ignored offline benchmark; embedded no-allocation target: 4 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | Pass, 4 passed, 0 ignored |
| `cargo xtask check desktop-native` | Pass; production/no-default, default, all-feature Tauri checks and `tauri-test` suite completed |
| `cargo xtask check all` | First attempt: exit 1 because the linked evidence file did not exist yet. After creating this file: pass, exit 0. Static architecture audit reported 67 advisory warnings; static, security, workspace Clippy/tests, Bun desktop checks, browser E2E, and desktop-native checks completed. |
| Offline A/B filter `cargo test ... --lib offline_waiter_guard_ab_benchmark -- --list` | Pass; exactly one ignored test is discoverable. Benchmark was not executed. |

The five reviewed `sky_player` production warnings are absent: both strict
Clippy configurations, the direct no-default production build, the actual
Tauri binary check, and `cargo xtask check desktop-native` completed without
Rust compiler warnings. The separate 67 architecture advisories are xtask
static-audit notices outside this warning set.

Development retries/failures: the first preflight test fixture had a
non-monotonic B timestamp; it was corrected to A Down at 0, B Down at 30 ms,
A Up at 80 ms, and B Up at 2 s, then the focused tests passed. The first
no-default strict Clippy run surfaced the inline observation enum lint described
above; the documented narrow expectation was added and the required command
passed. One early test-filter typo matched zero tests and was replaced by the
correct `due_down_owner_permit_contention_uses_outer_loop_retry_and_bounded_wait`
filter, which passed. The first `cargo fmt --check` printed the two formatting
changes that were then applied with `cargo fmt`; the final format checks passed.
The first `cargo xtask check all` attempt caught the missing evidence link; after
creating this file, the complete serial rerun passed. No failed test or command
is counted as passing evidence.

## Qualification limit

All dispatch and sender evidence here uses deterministic mock senders. No real
`SendInput`, Sky gameplay, interactive focus, ReceiveOnly run, or native
suspend/resume qualification was performed. These tests do not establish that
Sky received or played any note.

## Final checkpoint references

Implementation commit/tree/parent are listed at the top. The final evidence
commit/tree/parent, remote branch tip, and exact-HEAD hosted CI run/status are
recorded in the checkpoint handoff. The final serial `cargo xtask check all`
passed with exit 0 after the evidence link was added.

# Coordinator R3 P0 Keyboard Baseline

Refs [#431](https://github.com/pumni/Sky-Auto-Player/issues/431) and [#430](https://github.com/pumni/Sky-Auto-Player/issues/430).

P0 is characterization only. No production timing, lease, scheduling, focus, or cleanup policy was changed. No game process was opened. Native controls used the existing `SendInput` acceptance harness with its `ReceiveOnly` sink.

## Pinned evidence

- Run: `r3-p0-20261004T185505507Z-0ab4da0e`
- Implementation/source revision: `550fa1a6514a57e6e452895a1b88be8d79a6c8ef`
- Runtime/base revision: `dd915bf31e49e45315b8dbb427d0506046c876b7` (`origin/main` merge base)
- Release probe SHA-256: `825adf664e8be1651a96fb4704010e5830af2796b05116ecee6ad7722cbb4d24`
- Evidence commit: `5245909aeb6f204d8d6542f69f6f6ec3852e5d91` on `rt-r3/evidence/p0-550fa1a6514a`
- Runner manifest SHA-256: `26b82a139760e350ed1c5a090d4cb4b813fe5786cc8d5728c6915e04ff4f55fb`
- ZIP SHA-256: `e8f2fd99372fb8977061814a91b33c96b558df792194adc8941ceb460b5c5ca3` (886,571 bytes)
- Bundle: [pinned evidence run](https://github.com/pumni/Sky-Auto-Player/tree/5245909aeb6f204d8d6542f69f6f6ec3852e5d91/runs/r3-p0-20261004T185505507Z-0ab4da0e)
- Manifests: [outer manifest](https://github.com/pumni/Sky-Auto-Player/blob/5245909aeb6f204d8d6542f69f6f6ec3852e5d91/runs/r3-p0-20261004T185505507Z-0ab4da0e/manifest.json), [runner outputs](https://github.com/pumni/Sky-Auto-Player/blob/5245909aeb6f204d8d6542f69f6f6ec3852e5d91/runs/r3-p0-20261004T185505507Z-0ab4da0e/run/runner-outputs.json)
- Main reports: [contract vectors](https://github.com/pumni/Sky-Auto-Player/blob/5245909aeb6f204d8d6542f69f6f6ec3852e5d91/runs/r3-p0-20261004T185505507Z-0ab4da0e/run/contracts.json), [precision summary](https://github.com/pumni/Sky-Auto-Player/blob/5245909aeb6f204d8d6542f69f6f6ec3852e5d91/runs/r3-p0-20261004T185505507Z-0ab4da0e/run/summary.json)

The publisher archived only runner-declared outputs. The bundle contains 320 declared files (55,224,124 raw bytes), SHA-256 sums, child stdout/stderr, all 90 precision reports, and the three native sink case directories. Each archive part is capped at 8 MiB; this run fits in one ZIP part.

## Host and run setup

- Windows 11 Home build 26300, `x86_64-pc-windows-msvc`; AMD Ryzen 5 5500U with Radeon Graphics.
- `rustc 1.98.1`, `cargo 1.98.1`, Bun `1.4.0`, PowerShell `7.6.6`, Git `2.55.0.windows.5`.
- Active Windows power scheme: Balanced. WMI reported battery `DELL WV3K819`, status `2`, estimated charge `131`; the estimate is retained as reported and AC connection state was not separately captured.
- Source tree was clean at capture. Probe profile was release. Precision runs used host QPC at 10,000,000 Hz; contract vectors used synthetic QPC frequencies 1,000,000, 10,000,000, and 10,000,003 Hz.
- Focus was a test-support `focus_active=true` with deterministic `SessionTarget`. No game process or foreground-window activation was used. Precision transport was a deterministic mock, not a game/audio receipt measurement.
- Seed `1073`; two hidden PowerShell busy workers were present only for `cpu_contention` groups.
- Runner command: `pwsh -NoProfile -File scripts/run_rt_r3_evidence.ps1 -Stage baseline -OutputRoot .benchmarks/r3/p0`, with the temporary Bun 1.4.0 directory prepended to that process's `PATH`.
- The runner captured 97 child commands; all 97 exited zero. It rejected unimplemented stages explicitly. The `phase1` probe returned `BLOCKED-STAGE` and generated no evidence or PASS result.

## Contract vectors

### A — late same-key dispatch across manual pause/resume

The executable lifecycle probe reproduced the late next-Down result at all three QPC frequencies. On each vector, Down pre-call/completion was 100,000/100,000 µs; musical Up pre-call/completion was 117,167/117,217 µs; pause/resume was 117,300/118,300 µs. The next Down entered the sender at 118,300 µs although its required floor was 133,884 µs, 15,584 µs later.

The exact authored tail ends `D0,U17167,D34334,U51501,U51501`. The schedule compiler rejected it with `DuplicateSameTimestampUp { scan_code: 21, scheduled_us: 51501 }`. The probe therefore exercised the valid `D0,U17167,D34334` prefix only; it did not bypass the compiler or claim that the malformed full vector ran.

### B — enabled supervisor lease with delayed watchdog

The harness configured a 3,000,000 µs lease. With last progress at 1,000,000 µs and dispatch at 4,000,001 µs, the expired latch was still false and the prepared final Down was admitted with one sender attempt (`STALE_LEASE_ACCEPTED`). Equality at 3,000,000 µs and a fresh 1 µs age were recorded alongside it. This deterministic worker harness does not start the watchdog scheduler, so the result characterizes dispatch admission while the shared expiry latch remains false.

The native `supervisor-lease-expiry` control separately exercised the real supervisor path: it passed with `terminal_error=supervisor_lease_expired` and zero events at the `ReceiveOnly` sink.

### C — serial head-of-line floor

A Down completed at 110,000 µs. Its authored Up target was 117,167 µs, but the hold floor put the Up pre-call at 127,167 µs. B targeted 118,000 µs and its earliest pre-call was 127,167 µs, 9,167 µs late. The prepared cursor advanced from 2 to 3; this records serial admission and makes no reorder or catch-up claim.

### D — focus loss and ownership cleanup

While unfocused, the owned-key mask remained `1` and no cleanup send occurred. On modeled trustworthy restoration, the shared verified cleanup path sent one Up for scan code `21`; the active mask and release obligation then reached zero and one generation was cancelled. Terminal finalization while unfocused also emitted one ownership-scoped Up.

The focus-grace scheduler itself was not exercised by the deterministic probe. Restoration is modeled through the existing test-support transition and cleanup/reconciliation path; the native controls are separate evidence.

### E — empty cleanup

Requested and attempted masks were zero, with zero keyboard transport calls and no transport anomaly. The oracle passed.

## Precision baseline

Each of the nine declared workloads ran five times in each load mode. Every run had 1,000 warmup and 10,000 measured samples, for 90 runs, 90,000 warmup samples, and 900,000 measured samples total. All 18 workload/load cells were statistics-eligible with five clean runs each. The table shows the median of the five run-level admission P99 values and the min–max range of those five P99s, in integer microseconds.

| Workload | Quiet: median P99 (range) | CPU contention: median P99 (range) |
| --- | ---: | ---: |
| `down-1` | 3 (3–3) | 3 (2–3) |
| `down-5` | 2 (2–3) | 3 (3–3) |
| `down-15` | 5 (3–5) | 3 (2–3) |
| `mixed-1x1` | 1 (1–4) | 3 (1–3) |
| `mixed-2x3` | 4 (3–4) | 3 (2–3) |
| `mixed-7x8` | 3 (1–3) | 2 (2–3) |
| `up-only-1` | 0 (0–0) | 0 (0–0) |
| `up-only-5` | 0 (0–0) | 0 (0–0) |
| `up-only-15` | 0 (0–0) | 0 (0–0) |

Microsecond values are integer conversions of raw QPC ticks; zero-valued microsecond quantiles do not mean a precisely zero-duration operation. The raw tick samples, other percentiles, per-run maxima, counters, and statistics eligibility are in the pinned precision reports. P0 records baseline runs only; paired AB/BA comparison is reserved for P2.

## Native ReceiveOnly controls

All three controls used `TimingMarginUs=500` and the feature-gated release harness (SHA-256 `5b570835e93bd270bc3c786de9ee74bff0562b571ddc245f2775a890e3aab6c9`). The sink role was `ReceiveOnly`.

| Scenario | Run ID | Result | Sink events |
| --- | --- | --- | ---: |
| `canonical-single` | `native-case-20261005T015557-f69bcf25` | PASS, finished | 2 |
| `pause-resume` | `native-case-20261005T015600-1d89bcee` | PASS, finished | 4 |
| `supervisor-lease-expiry` | `native-case-20261005T015604-b903aa08` | PASS, watchdog expired | 0 |

The native sink logs and `rt-native-acceptance.jsonl` files are in the pinned evidence run under `run/native/`.

## Gates

Passed on the P0 implementation revision:

- `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core` — 73 passed.
- `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` — 248 passed, 1 ignored.
- `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib` — 368 passed.
- `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` — 23 passed.
- `cargo xtask check static` — PASS.
- `cargo xtask check all` — PASS, including workspace/all-features tests, all-target/all-feature Clippy, desktop checks, and Bun E2E.
- `git diff --check` — PASS.

No P1 work has started. No PR merge or issue closure has been performed.

READY FOR COORDINATOR REVIEW; NEXT PHASE NOT STARTED

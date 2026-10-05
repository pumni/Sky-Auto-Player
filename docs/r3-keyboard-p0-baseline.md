# Coordinator R3 P0 Keyboard Baseline

Refs [#431](https://github.com/pumni/Sky-Auto-Player/issues/431) and [#430](https://github.com/pumni/Sky-Auto-Player/issues/430).

P0 is characterization only. No production timing, lease, scheduler, focus, or cleanup policy changed. No game process was opened. Native controls used the existing SendInput acceptance harness with its ReceiveOnly sink.

## Pinned evidence

- Run: r3-p0-20261004T234223688Z-d88f1a19
- Implementation/source revision: 325f45e6089c46218e1ee988ff391643d83ad43a
- Runtime/base revision: dd915bf31e49e45315b8dbb427d0506046c876b7 (origin/main merge base)
- Release probe SHA-256: fe7ca9ba6b9abc26110de583127585b5c56ccfad8a2f40412f3567fdf98138ad
- Evidence commit: e8c49fac8ba5bebf02ef0b47c05f7908a4769a3d on rt-r3/evidence/p0-325f45e6089c
- Outer manifest SHA-256: 296024e46c366457504373043825364d13c5b322bd11345695cd35c7d29bc38c
- ZIP SHA-256: 4218cb9213114becfbb60f234716eb64bfa4a92fb3ff46afb36f40794d36bbf0 (1,088,971 bytes; one part)
- runner-outputs.json SHA-256 inside the ZIP: a239e419e5cc2a8fc32fbc199596bc414f8cd60259602eda81360abe60a98379
- files/contracts.json SHA-256 inside the ZIP: f865d41b6ee637fb02ab697905fe1030bf84d1b4b20fcd9bdf45d3d97a9c130d
- files/summary.json SHA-256 inside the ZIP: 89ee60ea4cf284603056a5c1aa252125c523db0a51c6fb6ff311e2a5eff147d6
- Pinned run: [evidence directory](https://github.com/pumni/Sky-Auto-Player/tree/e8c49fac8ba5bebf02ef0b47c05f7908a4769a3d/runs/r3-p0-20261004T234223688Z-d88f1a19)
- Pinned files: [outer manifest](https://github.com/pumni/Sky-Auto-Player/blob/e8c49fac8ba5bebf02ef0b47c05f7908a4769a3d/runs/r3-p0-20261004T234223688Z-d88f1a19/manifest.json), [SHA256SUMS.txt](https://github.com/pumni/Sky-Auto-Player/blob/e8c49fac8ba5bebf02ef0b47c05f7908a4769a3d/runs/r3-p0-20261004T234223688Z-d88f1a19/SHA256SUMS.txt), [evidence.zip](https://github.com/pumni/Sky-Auto-Player/blob/e8c49fac8ba5bebf02ef0b47c05f7908a4769a3d/runs/r3-p0-20261004T234223688Z-d88f1a19/archive/evidence.zip)

The archive contains 320 declared raw files totaling 55,368,576 bytes. runner-outputs.json, files/contracts.json, files/summary.json, precision reports, and native control directories are entries inside evidence.zip; they are not separate GitHub blobs at this pinned commit. This corrects the earlier nonexistent inner-file links. The earlier bundle remains immutable at [its pinned commit](https://github.com/pumni/Sky-Auto-Player/tree/5245909aeb6f204d8d6542f69f6f6ec3852e5d91/runs/r3-p0-20261004T185505507Z-0ab4da0e).

The outer manifest SHA256 recorded for this earlier run was calculated from CRLF working-tree bytes. Git normalized the committed text blob to LF, so that recorded outer manifest hash and its checksum row do not verify against the pinned Git blob. The ZIP and all 320 raw files remain intact; the corrected publisher verifies committed manifest and archive bytes before pushing a new append-only run.

## Host and run setup

- Windows 11 Home build 26300, x86_64-pc-windows-msvc; AMD Ryzen 5 5500U with Radeon Graphics.
- rustc and cargo 1.98.1, Bun 1.4.0, PowerShell 7.6.6, Git 2.55.0.windows.5; Balanced power scheme.
- The source tree was clean at capture. The probe used the release profile. Host QPC was 10,000,000 Hz; contract vectors used synthetic QPC frequencies 1,000,000, 10,000,000, and 10,000,003 Hz.
- Focus setup was test-support focus_active=true with a deterministic SessionTarget. No game process or foreground-window activation was used.
- The precision transport was a deterministic mock, not a game/audio receipt measurement. Two hidden PowerShell busy workers were used for CPU-contention runs.
- Command: pwsh -NoProfile -File scripts/run_rt_r3_evidence.ps1 -Stage baseline -OutputRoot .benchmarks/r3/p0. Bun 1.4.0 was prepended to that process PATH.
- The runner recorded 97 child commands; all 97 exited zero. Unimplemented stages fail explicitly; phase1 returned BLOCKED-STAGE without producing evidence or PASS.

## Review corrections R1-R5

### R1: due-aware physical-floor gate

The due-seam test helper accepts only synthetic wall QPC and recomputes the current production physical floor for the current prepared packet on every call. Before that floor it returns without entering the prepared suffix or changing sender attempts, cursor, generation accounting, or release obligations. It does not implement a second scheduler.

The earlier R1 probe reused the pre-cleanup floor when checking the resume call. That input masked the required counterexample, so its claim that the resumed Down waited until 133,884 us is invalid. The new baseline records the pre-cleanup window as a snapshot only, checks the real resume at 118,300 us against the recomputed post-cleanup floor, and retries at the current floor only if resume is still early. It separately compares the observed Down with the independent Up-completion-plus-frame requirement. This is characterization evidence; production timing and cleanup behavior are unchanged.

### R2: native-admitted fixture configuration

Every R3 fixture now materializes frame/base hold 16,667 us, timing margin 500 us, effective H 17,167 us, authored minimum release gap 17,167 us, and physical same-key G 16,667 us. Production validate_native_timing_contract and validate_native_schedule_timing_with_release_gap validators run before prepared-stream construction, including tick-domain admission. JSON records the actual config and converted ticks.

A uses four valid authored events. The duplicate-Up vector is retained only as a rejected malformed-input control. The odd-frequency vector carries its required 1 us feasibility surplus. All A and C configuration/admission checks report PASS.

### R3: enabled lease in precision measurements

All precision workloads use an enabled 3,000,000 us lease, materialized as 30,000,000 QPC ticks at the 10,000,000 Hz host frequency. Fresh progress is published after setup and outside each measured interval. The watchdog is not started in the microbenchmark. The runner fails closed on missing or mismatched lease, timing, or progress metadata.

The sample-start QPC is taken immediately before prepared-dispatch entry, before modifier/late admission. The backend samples the authoritative sender pre-call before entering the deterministic mock emitter. The mock samples completion inside that emitter, so the pre-call-to-completion delta includes test-seam/QPC overhead and does not measure SendInput execution, device receipt, or game/audio receipt.

### R4: valid evidence URLs and hashes

The report links only to paths present at the pinned evidence commit: the outer manifest, checksums file, and ZIP. Inner report files are identified by their archive entry names and hashes above. The large raw output bundle stays on the evidence branch; the earlier bundle has not been replaced.

### R5: distinct B dispositions

The delayed-watchdog vector used an enabled lease with last progress at 1,000,000 us, timeout 3,000,000 us, and dispatch at 4,000,001 us. With expiry not latched, the prepared Down was admitted in one sender attempt and is labeled STALE_LEASE_ACCEPTED. At exact equality, the result is AT_TIMEOUT_ALLOWED with one attempt. With fresh progress age 1 us, the result is FRESH_PROGRESS_ALLOWED with one attempt. The watchdog is deliberately unscheduled, so this is a final-gate characterization.

## Contract vectors

### C: serial head-of-line floor

A Down completed at 110,000 us. Its authored Up target was 117,167 us, but the hold floor placed the Up pre-call at 127,167 us. B targeted 118,000 us; its earliest pre-call was 127,167 us, 9,167 us late. The prepared cursor advanced from 2 to 3. This records serial admission and makes no reorder or catch-up claim. Native and tick-domain admission passed.

### D: focus loss and ownership cleanup

While unfocused, the owned-key mask remained 1 and no cleanup send occurred. On modeled trustworthy restoration, the shared verified cleanup path sent one Up for scan code 21; the active mask and release obligation reached zero and one generation was cancelled. Terminal finalization while unfocused also emitted one ownership-scoped Up.

The focus-grace scheduler itself was not exercised by the deterministic probe. Restoration is modeled through the existing test-support transition and cleanup/reconciliation path. Native controls are separate evidence.

### E: empty cleanup

Requested and attempted masks were zero, with zero keyboard transport calls and no transport anomaly. The oracle passed.

## Precision baseline

Nine workloads ran five times in each of two load modes. Each run had 1,000 warmup and 10,000 measured samples: 90 runs, 90,000 warmup samples, and 900,000 measured samples. All 18 workload/load cells were eligible, each with five clean runs. The table reports the median of the five run-level admission P99 values and their min-max range, in integer microseconds.

| Workload | Quiet: median P99 (range) | CPU contention: median P99 (range) |
| --- | ---: | ---: |
| down-1 | 2 (2-4) | 3 (3-4) |
| down-5 | 3 (3-5) | 3 (3-4) |
| down-15 | 3 (2-3) | 3 (2-4) |
| mixed-1x1 | 1 (1-3) | 2 (1-3) |
| mixed-2x3 | 3 (3-4) | 3 (2-3) |
| mixed-7x8 | 2 (1-3) | 3 (1-3) |
| up-only-1 | 0 (0-0) | 0 (0-0) |
| up-only-5 | 0 (0-0) | 0 (0-0) |
| up-only-15 | 0 (0-0) | 0 (0-0) |

Microsecond values are integer conversions of raw QPC ticks; zero-valued quantiles do not mean a precisely zero-duration operation. Raw tick samples, p50/p95/max, per-run details, counters, and eligibility are in the pinned archive. P0 contains baseline runs only; paired AB/BA comparison is reserved for P2. No P2 performance acceptance is claimed.

## Native ReceiveOnly controls

The release harness and sink self-test passed. Each control reported matching ReceiveOnly identity, event log ID, contiguous sequence, and report window.

| Scenario | Run ID | Result | Sink events |
| --- | --- | --- | ---: |
| canonical-single | native-case-20261005T064321-180da2ca | PASS, finished | 2 |
| pause-resume | native-case-20261005T064325-b711544a | PASS, finished | 4 |
| supervisor-lease-expiry | native-case-20261005T064329-8a86ff3f | PASS, watchdog expired | 0 |

The zero-event supervisor case verifies its rejection/cleanup control path. These controls do not replace A/B vectors or prove game/audio receipt.

## Gates

Focused tests and local checks passed on implementation revision 325f45e6089c46218e1ee988ff391643d83ad43a:

- cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core — PASS, 73 tests.
- cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support — PASS, 248 passed, 1 ignored.
- cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib — PASS, 371 tests.
- cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc — PASS, 23 tests.
- cargo xtask check static — PASS; 31 allowlisted architecture warnings.
- cargo xtask check all — final full rerun PASS, including desktop checks and Bun E2E. The first `rtk cargo xtask check all` exited 1: `sky_desktop_shell_lib` reported 228 passed and 1 failed in `power_lifecycle::tests::stable_callback_context_routes_current_epoch_and_ignores_retired_epoch`, at `desktop/src-tauri/src/power_lifecycle.rs:861:9`. The failing assertion was `!resumed.suspended && resumed.down_blocked`; the output did not print the observed field values. Its targeted rerun and a later full rerun passed. No root cause was established.
- PowerShell parser validation of scripts/run_rt_r3_evidence.ps1 — PASS.
- Baseline runner — PASS; 97/97 child commands exited zero, including release native acceptance build, sink self-test, and all three native scenarios.
- git diff --check — PASS.

The earlier result above belongs to implementation revision 325f45e6089c46218e1ee988ff391643d83ad43a. After merging #439 and applying the current P0 corrections, `rtk cargo xtask check all` reached the desktop library tests twice and exited 1 both times: 228 passed and `power_lifecycle::tests::stable_callback_context_routes_current_epoch_and_ignores_retired_epoch` failed at `desktop/src-tauri/src/power_lifecycle.rs:861:9` on `!resumed.suspended && resumed.down_blocked`. The output does not print either observed flag value. The targeted command `rtk cargo test --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test --locked power_lifecycle::tests::stable_callback_context_routes_current_epoch_and_ignores_retired_epoch` passed once; workspace Rust checks, Bun check/E2E, Clippy, and static passed. No root cause is established. This remains a failing full gate and is reported without a claim that the failure is transient.

GitHub exact-head CI is recorded in the #437 PR body and the #431 checkpoint. P0 remains active pending coordinator review. No merge, issue closure, or P1 work is authorized here.

READY FOR COORDINATOR REVIEW; NEXT PHASE NOT STARTED

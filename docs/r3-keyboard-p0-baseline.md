# Coordinator R3 P0 Keyboard Baseline

Refs [#431](https://github.com/pumni/Sky-Auto-Player/issues/431) and [#430](https://github.com/pumni/Sky-Auto-Player/issues/430).

P0 is characterization only. No production timing, lease, scheduler, focus, or cleanup policy changed. No game process was opened. Native controls used the existing SendInput acceptance harness with its ReceiveOnly sink.

## Pinned evidence

- Run: r3-p0-20261005T024258505Z-700aaab7
- Implementation/source revision: 458ee68aa40ad48a9805b07b62afaec2e0ba58a5
- Runtime/base revision: 1574aa267b8364b1930bd6abc7baa36636d33843 (verified #439 merge on origin/main)
- Release probe SHA-256: 309de78a9f6847ad2097c5915052b99e130476259e1beac8980efb912666c22e
- Evidence commit: 596664e809b69318eb216d25b9bed680446641e3 on rt-r3/evidence/p0-458ee68aa40a
- Outer manifest SHA-256: ca4904e3114d77f2a7abf769b3a3701954297a803629cd16f6768999a4aa5fbb
- ZIP SHA-256: daaa6881c724a2319a43a24e469ea70d27c2472b263cd60a513421086a187e7c (1,170,232 bytes; one part)
- runner-outputs.json SHA-256 inside the ZIP: 7277874d3eb4b8258d2dbd6db24c6e89beb7cb3fe926b0609ed8752f0bfa7b62
- files/contracts.json SHA-256 inside the ZIP: 39179a62021b870bbe78e95182882ad66aeeff2476eb574f5a27e5b8b80ab1bd
- files/summary.json SHA-256 inside the ZIP: 0a3c6b6ff0e2954dec6a48c0800b07d062c088fe4eb931c1519f136e68339b2c
- Pinned run: [evidence directory](https://github.com/pumni/Sky-Auto-Player/tree/596664e809b69318eb216d25b9bed680446641e3/runs/r3-p0-20261005T024258505Z-700aaab7)
- Pinned files: [outer manifest](https://github.com/pumni/Sky-Auto-Player/blob/596664e809b69318eb216d25b9bed680446641e3/runs/r3-p0-20261005T024258505Z-700aaab7/manifest.json), [SHA256SUMS.txt](https://github.com/pumni/Sky-Auto-Player/blob/596664e809b69318eb216d25b9bed680446641e3/runs/r3-p0-20261005T024258505Z-700aaab7/SHA256SUMS.txt), [evidence.zip](https://github.com/pumni/Sky-Auto-Player/blob/596664e809b69318eb216d25b9bed680446641e3/runs/r3-p0-20261005T024258505Z-700aaab7/archive/evidence.zip)

The archive contains 320 declared raw files totaling 55,390,917 bytes. runner-outputs.json, files/contracts.json, files/summary.json, precision reports, and native control directories are entries inside evidence.zip; they are not separate GitHub blobs at this pinned commit. The publisher read and SHA-256 checked the committed manifest and archive blob bytes before pushing; the returned manifest hash is for the verified Git blob. This corrects the earlier nonexistent inner-file links and the newline-corrupted outer hash. The superseded run remains immutable at [its pinned commit](https://github.com/pumni/Sky-Auto-Player/tree/e8c49fac8ba5bebf02ef0b47c05f7908a4769a3d/runs/r3-p0-20261004T234223688Z-d88f1a19); its previously reported outer hash 296024e46c366457504373043825364d13c5b322bd11345695cd35c7d29bc38c was computed from CRLF working-tree bytes and does not match the normalized Git blob. Its ZIP and 320 raw files remain intact. The older bundle remains at [its separate pinned commit](https://github.com/pumni/Sky-Auto-Player/tree/5245909aeb6f204d8d6542f69f6f6ec3852e5d91/runs/r3-p0-20261004T185505507Z-0ab4da0e).

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

The earlier R1 probe reused the pre-cleanup floor when checking the resume call. That input masked the required counterexample, so its claim that the resumed Down waited until 133,884 us is invalid. In the corrected run, the resume call is at 118,300 us. The seam recomputes the post-cleanup production floor (118,034 us at 1,000,000 and 10,000,000 Hz; 118,036 us at 10,000,003 Hz), so resume is due and the prepared suffix dispatches immediately at 118,300 us. The independent Up-completion-plus-frame requirement remains 133,884 us (1,338,840 ticks at 10 MHz; 1,338,844 ticks at 10,000,003 Hz). Thus the normal-gap contract is REPRODUCED/false while due-seam integrity is true at all three frequencies. The pre-cleanup floor remains evidence only. The no-pause negative control at 118,300 us is still not due at 133,884 us and leaves sender attempts, cursor, generation accounting, and release obligation unchanged. Final Up dispatches at its current legal production floor; accounting is 2 activated / 2 released, with zero obligation. This is characterization evidence; production timing and cleanup behavior are unchanged.

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
| down-1 | 4 (4-4) | 4 (4-7) |
| down-5 | 3 (3-4) | 4 (4-4) |
| down-15 | 4 (3-5) | 5 (4-11) |
| mixed-1x1 | 3 (3-3) | 4 (3-4) |
| mixed-2x3 | 3 (3-4) | 4 (4-21) |
| mixed-7x8 | 4 (3-4) | 3 (3-4) |
| up-only-1 | 0 (0-0) | 0 (0-0) |
| up-only-5 | 0 (0-0) | 0 (0-0) |
| up-only-15 | 0 (0-0) | 0 (0-0) |

Microsecond values are integer conversions of raw QPC ticks; zero-valued quantiles do not mean a precisely zero-duration operation. Raw tick samples, p50/p95/max, per-run details, counters, and eligibility are in the pinned archive. P0 contains baseline runs only; paired AB/BA comparison is reserved for P2. No P2 performance acceptance is claimed.

## Native ReceiveOnly controls

The release harness and sink self-test passed. Each control reported matching ReceiveOnly identity, event log ID, contiguous sequence, and report window.

| Scenario | Run ID | Result | Sink events |
| --- | --- | --- | ---: |
| canonical-single | native-case-20261005T094407-0f097726 | PASS, finished | 2 |
| pause-resume | native-case-20261005T094411-7d4e2a91 | PASS, finished | 4 |
| supervisor-lease-expiry | native-case-20261005T094416-1bdfe690 | PASS, watchdog expired | 0 |

The zero-event supervisor case verifies its rejection/cleanup control path. These controls do not replace A/B vectors or prove game/audio receipt.

## Verification and checkpoint

- Focused Rust suites: `sky_dispatch_core` PASS (73); `sky_dispatch_win32 --features test-support` PASS (248 passed, 1 ignored); `sky_player --features test-support --lib` PASS (371); `rt_dispatch_no_alloc` PASS (23). The final due-seam assertion was also covered by both successful workspace Rust stages inside `check all`.
- `cargo xtask check static` PASS, with 31 allowlisted architecture warnings. `cargo fmt --manifest-path rust/Cargo.toml --all -- --check`, `git diff --check`, and PowerShell parser validation for the runner and publisher PASS.
- Temporary publisher tests with `core.autocrlf=true` and `false` plus `* text=auto` verified byte-identical committed manifest, checksum, and archive contents. The final publisher also verified the committed outer manifest and archive blob bytes before push and returned the manifest SHA from that verified blob.
- Final baseline run `r3-p0-20261005T024258505Z-700aaab7`: 97/97 child commands exited zero, 90 precision runs yielded 900,000 samples with no failed samples and 18/18 eligible cells, and all three native ReceiveOnly controls passed. The earlier unpublished native attempt stopped when the exact sink HWND lost foreground; the user confirmed an accidental focus change, then the fresh run passed. An earlier publisher attempt was blocked before push and is not part of the pinned evidence.
- `cargo xtask check all` remains FAIL on the corrected P0 code. Two full runs exited 1 in `sky_desktop_shell_lib`: 228 passed and `power_lifecycle::tests::stable_callback_context_routes_current_epoch_and_ignores_retired_epoch` failed at `desktop/src-tauri/src/power_lifecycle.rs:861:9` on `!resumed.suspended && resumed.down_blocked`. The output did not print the observed flag values. The targeted command `cargo test --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test --locked power_lifecycle::tests::stable_callback_context_routes_current_epoch_and_ignores_retired_epoch` passed once. Workspace Rust tests, Clippy, Bun check/E2E, and static passed; no root cause is established. This remains an explicit full-gate failure, with no claim that it is transient.

The measured source revision is 458ee68aa40ad48a9805b07b62afaec2e0ba58a5. This report update is a separate documentation-only commit after measurement; the source SHA and release binary SHA above identify the code that produced the evidence. [GitHub CI run 37256459701](https://github.com/pumni/Sky-Auto-Player/actions/runs/37256459701) for the measured source revision completed successfully, including the required CI gate. The local full `check all` failures above remain recorded; the differing local and CI outcomes are unresolved, with no root-cause claim.

P0 remains active for coordinator review. PR #437 remains Draft and unmerged; issue #431 remains open. No merge or P1 work is authorized.

P0 CHECKPOINT SUBMITTED; COORDINATOR ACCEPTANCE PENDING; NEXT PHASE NOT STARTED

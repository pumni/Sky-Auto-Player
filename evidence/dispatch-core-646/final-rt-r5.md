# FINAL-RT-R5 — local implementation evidence

This report covers the bounded L23 implementation batch on `perf/dispatch-core-20261109`, starting at `331269e52b209be96366ae97eea9fa74c755a4ad`. It records implementation and local verification for leader review. It does not claim native qualification, release qualification, or SOURCE ACCEPTED.

## P1 — startup and shutdown lifecycle

Shutdown now serializes with the final pre-arm step. The service latches its sticky shutdown cancellation before compilation can publish a player; when a player is already published but not armed, shutdown latches Quit while holding the arm gate. A start racing with accepted shutdown cannot arm a fresh session, and the sticky shutdown state prevents restart.

The desktop test `shutdown_after_compilation_before_publication_retires_start_without_dispatch` holds compilation at the pre-publication barrier, accepts shutdown, then verifies no player publication/dispatch, activity retirement, and no restart. `stop_skip_and_shutdown_cancel_the_published_pre_arm_session` covers the published/pre-arm path. These are deterministic Rust/Tauri test-support lifecycle checks; they do not send physical input. Stop semantics remain unchanged: there is no Stop-before-session API.

## P2 — Up-slot conservation and observer work

Telemetry schema is now 25. Final Up-slot accounting checks:

```text
expected = captured sidecar records + dropped by cause + not captured by mode
```

Observer-abort loss has its own counter. Queue, pool-storage, telemetry-ring, sidecar-capacity and observer-abort loss are distinct; capture-off records are counted as not captured. Finalization records `valid` only when the checked sum matches. Overflow, mismatch, or incomplete observer accounting is `invalid`, never silently balanced. Observer shutdown joins and drains queued work, and counts stranded Up slots after abort. Packet summaries still do not claim per-key p95/p99.

The producer enqueue path no longer samples queue occupancy with `len()`. The observer consumer owns high-watermark sampling; the producer path retains fixed-capacity, no-allocation queue behavior. The full player suite covers observer abort before and after partial drain, queue pressure, Ring/Off capture, pool/sidecar loss and JSON accounting. The separate RT no-allocation integration target passed 29 tests.

## D — admission budget

The fixed 512 MiB policy remains `536,870,912` bytes. The one-million-action valid single-key test compiled within the compiler-phase bound (peak capacity estimate `481,000,758` bytes), then rejected combined coordinator/executor admission at `741,502,832` bytes against the same policy. The focused test passed and its captured reason is `combined schedule, coordinator and executor capacity ... exceeds dispatch budget of 536870912 bytes`.

These values are calculated size/capacity and lifetime bounds. Complete application peak and Windows process RSS were not measured; the rejection is not an OOM result. The test log retains source (`73,000,000` B), compiler input (`80,000,000` B), compiler temporary (`438` B), compiled schedule (`328,000,104` B), and policy values.

## Actual B-worker stale-dispatch timing

The raw samples are in [`stale-dispatch-timing-matrix.json`](./stale-dispatch-timing-matrix.json), SHA-256 `13081a28eb2c7973183a2488b0db021b12462599be7901d9bab909e2a0e68669`. Scope: actual B worker, test-support mock sender, production-calibrated Frozen waiter, 10 MHz QPC. Nearest-rank quantiles are microseconds. `deadline→pre-call` uses the physical deadline and true sender pre-call timestamp. Missing wake→pre-call values remain missing; deadline latency is not used to fill them. This matrix is not native qualification (`qualification=false`).

| Stale metadata | Repetitions | Wake→pre-call n / missing; p50 / p95 / p99 / max | Deadline→pre-call n / missing; p50 / p95 / p99 / max | Stale scans total; after attributed wake / without attributed wake |
| ---: | ---: | --- | --- | ---: |
| 0 | 100 | 100 / 0; 30 / 41 / 50 / 50 | 100 / 0; 30 / 45 / 50 / 1,604 | 0; 0 / 0 |
| 1 | 100 | 100 / 0; 56 / 73 / 80 / 88 | 100 / 0; 56 / 76 / 92 / 282 | 100; 100 / 0 |
| 32 | 100 | 1 / 99; 40 / 40 / 40 / 40 | 100 / 0; 162 / 279 / 462 / 484 | 3,200; 3,168 / 32 |
| 33 | 100 | 0 / 100; unavailable | 100 / 0; 159 / 270 / 334 / 483 | 3,300; 3,200 / 100 |
| 10,000 | 20 | 0 / 20; unavailable | 20 / 0; 22,012 / 25,209 / 27,310 / 27,310 | 200,000; 640 / 199,360 |

## Handoff benchmark

The full `rt_handoff_bench` ran 6 policies × 9 workloads = 54 cells at 100 iterations per cell. It used `HybridWaiter::production` with deterministic mock transport and production-calibrated wait policy; it did not use production `SendInput`. The raw stdout/stderr are retained outside the checkout. The compact 54-cell result is [`handoff-benchmark-summary.json`](./handoff-benchmark-summary.json), SHA-256 `e7725536163b50fd8e4779bc5f3b890e2448eb51e25499e5bd598b1dc844c693`.

The table summarizes `dispatch_start_error_us`: p50 is the min–max of the nine per-workload cell medians, and p95/p99/max are the worst of the nine per-cell summaries (not pooled quantiles). The last column is the worst per-cell p95 for wake→final-policy. CPU is process CPU duty; spin duty is the range across workload cells.

| Policy | p50 range | Worst-cell p95 / p99 / max (µs) | Wake→final-policy worst p95 (µs) | CPU duty | Spin duty range (ppm) |
| --- | ---: | ---: | ---: | ---: | ---: |
| fixed_spin_1000us | 6–7 | 14 / 269 / 5,162 | 10 | 4.86% | 32,751–65,370 |
| fixed_spin_1500us | 5–8 | 14 / 41 / 2,676 | 12 | 7.45% | 56,422–108,658 |
| fixed_spin_250us | 48–139 | 680 / 891 / 3,477 | 13 | 2.05% | 991–3,666 |
| fixed_spin_400us | 8–19 | 408 / 3,745 / 5,054 | 12 | 1.44% | 3,330–9,078 |
| fixed_spin_700us | 6–8 | 71 / 1,704 / 4,264 | 11 | 2.79% | 15,117–37,618 |
| production_adaptive_spin (benchmark mode only) | 6–8 | 20 / 1,295 / 1,735 | 14 | 5.57% | 30,100–67,948 |

Each policy had 900 waits, 900 packet observations, 900 wait observations and 900 expected spin observations; observed queue drops, spin drops and missing spin observations were zero. The run exited 0 with `acceptance_clean=true`, but `statistics_eligible=false` at 100 iterations and `sender_cutoff_clean=false` because the sender is a test-support mock. The requested 10,000-iteration matrix was not run: linear extrapolation from the measured 119,037 ms full run is about 3 h 18 min 24 s. This estimate is not a measurement. Shipping Frozen wait and all acceptance thresholds remain unchanged.

## Local verification

Implementation tests and gates ran before this evidence report was added; the focused documentation-link test ran afterward. Per-command exit, duration, environment, raw log names and stdout/stderr SHA-256 are recorded in [`local-checks.json`](./local-checks.json), SHA-256 `678aeb07e086a411d81ab24d7ed0cfe974ec1d50b2d89e32afe966df119184fe`. Raw command logs remain under `%TEMP%\sky-auto-player-final-rt-r5-evidence-final`; they are not copied into the repository.

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo fmt --all -- --check` | 0 | pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | pass |
| `cargo test -p sky_dispatch_core` | 0 | 123 unit tests plus 4 integration tests passed |
| `cargo test -p sky_dispatch_win32` | 0 | 249 tests passed |
| `cargo test -p sky_player --features test-support` | 0 | 502 passed, 1 ignored; RT no-allocation binary 29 passed |
| `cargo test -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | 29 passed |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --features tauri-test` | 0 | 330 passed, 4 ignored |
| `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` | 0 | pass |
| `cargo xtask check all` (repository root) | 0 | full local gate passed, including workspace tests, E2E and Tauri checks |
| `cargo test -p sky_xtask current_repository_docs_integrity_passes` | 0 | focused documentation link/integrity test passed after the report and links were added |
| Focused one-million admission test with `--nocapture` | 0 | 1 passed; raw stdout SHA-256 `2b34c0a85651d4569ab3f8158d0ecf74a62e975d221e4a33212ed72de5038630`, stderr SHA-256 `28f660e64ea857721eceda3f8785b3deb13eb91d3de6a48c3a8a051c3808c645` |

The separate default-profile `bun run tauri dev` audit launched the desktop shell and its main window, then was intentionally stopped with Ctrl+C. Its exit was 1, not a successful process exit. Captured stdout contained Vite readiness only and stderr was empty; no `warning:`, `unused`, `deprecated` or `error:` lines were present in those captured streams. This is not a claim that all compiler warnings are absent. The first audit attempt was also interrupted before the native window launched; both raw captures and hashes are in `local-checks.json`.

## Native qualification and review state

No identity-verified interactive Sky / ReceiveOnly session or valid immutable manifests for this source were available. Native Compare/Final remains pending; no mock, headless run, or local desktop launch is presented as native PASS. No memory budget, Stop semantics, Frozen wait policy, release criterion, or benchmark threshold was changed. This batch is ready for leader review only.

# 08 — Test, differential verification và benchmark

> HISTORICAL: This migration test plan predates the frozen Rust golden corpus.
> The current production contract is Rust-only; see `README.md`.

## 1. Test pyramid

### Pure Rust unit tests

- generation compile/pairing;
- coordinator transitions;
- pause overlap accounting;
- estimator exact numeric behavior;
- send retry state machine với fake native API;
- wait decision logic;
- telemetry capacity/counters;
- lifecycle state transitions.

### Property tests

Dùng `proptest`:

- random ordered actions;
- random partial-send scripts;
- random pause/focus/command transitions;
- assert invariants trong `02`;
- no overflow/negative elapsed;
- terminal counts total.

### Differential tests

Một harness chạy Python oracle và Rust candidate trên cùng:

```text
actions
config
clock ticks
SendInput return sequence
focus timeline
command timeline
```

Compare:

- dispatch order;
- requested/sent/skipped scan codes;
- completion-relative release times;
- runtime outcomes;
- generation status counts;
- abort reasons;
- estimator state;
- terminal result.

Cho phép khác ở implementation-only IDs/pointers, không cho phép tolerance timing trong fake-clock mode.

### Windows integration

- real QPC/timer wake distribution;
- SendInput to project-owned receiver window;
- focus event wake;
- MMCSS acquire/fallback;
- GetAsyncKeyState release verification;
- extension wheel import CPython 3.14t;
- frozen PyInstaller selftest.

## 2. Required scripted send cases

| Case | Native returns | Expected |
|---|---|---|
| down full | `[3]` | sent 3, one call |
| down recovered | `[2,1]` | sent 3, retry 1, no sleep |
| down persistent zero | `[0,0]` | sent 0, dropped 3, two calls, no sleep |
| down partial tail | `[1,1]` for n=3 | sent prefix 2, dropped 1 |
| up split progress | `[1,1,1]` | sent all, no error |
| up zero then progress | `[0,1,2]` | sleep once, reset zero counter |
| up terminal failure | `[0,0,0]` | error after third zero |
| duplicate down | no native call | skipped duplicate |
| already-up | no native call | skipped duplicate |

## 3. Existing Python tests to keep as contract

At minimum chạy và không rewrite expectation để “fit Rust”:

- `test_core_send_overhaul_invariants.py`
- `test_runtime_dispatch.py`
- `test_dispatch_fidelity_refactor.py`
- `test_dispatch_fidelity_edges.py`
- `test_phase1_correctness.py`
- `test_phase4_lifecycle.py`
- `test_core_boundary.py`
- `test_send_diagnostics.py`
- engine/supervisor/UI playback tests.

Khi adapter thay object types, chỉ sửa fixture/construction ở seam; không sửa semantic assertions.

## 4. Golden corpus

Sinh golden artifacts từ baseline commit:

```text
tests/golden_rust_migration/<song>/<profile>/<fps>/
  actions.json
  runtime_trace.json
  summary.json
```

Không commit raw wall-clock samples như exact golden. Fake clock traces exact; real timing benchmark lưu statistical baseline riêng.

## 5. Performance measurements

Không dùng tiêu chí “5x” chung chung. Dispatch bị giới hạn bởi OS wait/syscall; target đúng là tail fidelity và allocation.

### Micro

- schedule compile throughput/peak memory;
- coordinator next deadline/pop/drain;
- packet lookup/build;
- healthy SendInput wrapper overhead excluding syscall fake;
- QPC call/spin loop overhead;
- snapshot read/write;
- PyO3 command call.

### Real-time

- sender `visible_lateness_us` p50/p95/p99/max;
- send_duration_pure;
- pre_send_spin;
- command acknowledgement;
- focus-loss-to-release latency;
- timer wake error;
- CPU usage in long gaps;
- allocation count after start;
- RSS across many songs.

## 6. Acceptance thresholds

Parity phase:

- zero semantic diff on fake-clock corpus;
- no regression >10% in p99 sender completion error against same machine/profile, unless statistically explained;
- healthy post-prepare hot path: zero heap allocation per single-note dispatch; chord path bounded/inline;
- no lock acquisition in worker healthy send path except OS wait primitives;
- no Python runtime attach/callback on worker;
- command p99 under 20 ms in event and poll modes;
- focus transition wake under current 20–50 ms sampling plus immediate event;
- no leaked handles/threads across repeated plays;
- no stuck keys in fault-injection release suite.

Performance claims require machine/config/sample count in report.

## 7. Tooling gates

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
uv run pytest -q
uv run python scripts/measure_dispatch_tail.py
```

Windows-only tests marker/feature phải rõ; non-Windows core CI vẫn chạy.

## 8. Sanitizers/verification

- Miri cho pure core pieces khi feasible;
- loom chỉ khi custom concurrency protocol cần model; không thêm nếu channel/atomics đơn giản;
- Windows Application Verifier/handle counters cho integration;
- audit `cargo deny`/`cargo audit` theo repo policy;
- inspect binary imports để chắc không kéo API bị cấm.

# Dispatch Core & Windows Input Refinement Plan

> **Status:** PROPOSED — EVIDENCE RECORDED; CONDITIONAL CANDIDATES NOT SHIPPED
> **Ngày lập:** 2026-07-28  
> **Snapshot được review:** `main` tại
> `e9f4fa422cfec95f61c94ba768979d7e84554afd`  
> **Phạm vi:** `PlaybackEngine → PlaybackSupervisor → DispatchLoop →
> WinSendInputBackend → user32.SendInput`, adaptive wait/lead, calibration,
> priority, telemetry, prewarm/cache và GC lifecycle.  
> **Tính chuẩn tắc:** đây là proposal/working plan. `AGENTS.md`, `SECURITY.md`,
> `pyproject.toml` và các tài liệu P2 được liệt kê trong
> [`docs/INDEX.md`](../INDEX.md) luôn thắng khi có xung đột.

Plan này chuyển review tĩnh thành chuỗi thay đổi nhỏ, có thể kiểm chứng và
rollback độc lập. Việc một hạng mục có mặt trong plan **không** đồng nghĩa AI
được phép triển khai nó nếu benchmark/decision gate của hạng mục chưa đạt.

## 1. Kết quả mong muốn

Sau khi hoàn tất các phase bắt buộc và chỉ các phase tùy chọn đã vượt gate:

1. Calibration không còn race giữa `SendInput` return timestamp và `WM_INPUT`
   callback trên CPython 3.14 free-threaded.
2. Adaptive-spin preflight của threaded playback được đo trên chính dispatch
   thread, sau khi timer/priority/EcoQoS policy thực tế đã được áp dụng và trước
   final epoch anchor.
3. Mid-song reprobe vẫn giữ đủ sample và công thức hiện tại nhưng không độc
   chiếm dispatch thread trong một block khoảng 16 ms.
4. Calibration được gọi đúng là host-side injected Raw Input delivery proxy,
   không bị quảng bá thành game/audio-onset truth; freshness policy chỉ được
   ship sau khi có evidence và fallback safety được quyết định rõ.
5. Mọi tối ưu `_emit`, telemetry, prewarm, lead cache, GC hoặc power policy đều
   có baseline, benchmark gate, rollback trigger và không được nhận vơ speed-up
   end-to-end từ microbenchmark sender.
6. Normative docs mô tả đúng code đã ship, đặc biệt completion-anchor,
   `min_hold_margin_us`, adaptive probe/reprobe và metric semantics.
7. Không có full rewrite, không đổi input technology và không làm yếu release,
   cleanup, focus hay partial-send safety.

## 2. Guardrail bất biến

### 2.1 P0 security

- Chỉ Windows `SendInput` được dùng để mô phỏng input.
- Không đọc memory game, không hook, DLL injection, debugger attach, process
  tampering, driver input, `PostMessage`, anti-cheat evasion hoặc game-file
  modification.
- Không thêm `python-keyboard`, `pynput`, `SetWindowsHookEx` hay third-party
  keyboard module.
- Không giảm immediate one-retry cho partial note-on.
- Không giảm release retry, three-pass cleanup, `GetAsyncKeyState`
  verification, watchdog hoặc focus-loss abort.
- Mọi thay đổi chạm `platform/win32/inputs.py`, calibration injection hoặc
  `WinSendInputBackend` phải chạy security audit.
- Không sửa `scripts/audit_security_mandates.py` hoặc
  `.config/security_audit_baseline.json` nếu chưa có phê duyệt riêng.

### 2.2 Architecture

- `domain/` và `orchestration/core/` tiếp tục pure, không import Win32,
  `ctypes`, `SendInput`, wall clock hoặc concrete platform type.
- `platform/` là nơi duy nhất chứa Win32/`ctypes`.
- `infrastructure/` có thể bridge platform nhưng không được kéo ngược vào
  `domain/`.
- `DispatchLoop` chỉ nhận protocol/callback/primitives đã inject. Adaptive
  probe không phải lý do để import `HybridWaitableTimer` hoặc Win32 vào core.
- Python pin vẫn là cặp `.python-version` và `requires-python`; plan này không
  được thay đổi cặp đó.
- Không tự động nâng lên `TIME_CRITICAL`. Chính sách `auto` hiện tại tiếp tục
  dừng ở MMCSS/HIGHEST trừ khi một task riêng được phê duyệt.

### 2.3 Timing và musical safety

- Một chord bình thường vẫn là một `SendInput` batch.
- Timestamp completion vẫn là thao tác Python đầu tiên sau native return.
- Sender completion là proxy, không phải xác nhận game đã poll/render/play
  audio.
- Không thay `min_hold_frames`, same-key feasibility, conflict policy hoặc
  release floor chỉ để cải thiện telemetry.
- Không gộp behavior change với refactor, schema migration hoặc performance
  cleanup không liên quan.

## 3. Baseline và giới hạn bằng chứng

Snapshot review đã có các bằng chứng sau:

- Security audit: green.
- Free-threaded audit: CPython 3.14.3, `Py_GIL_DISABLED=1`, runtime GIL
  disabled.
- 108 targeted tests cho adaptive spin/reprobe, calibration, lead cache,
  dispatch fidelity, memory hygiene, HUD metric và core boundary: green.
- Microbenchmark tại snapshot:
  - cached `INPUT[]` lookup khoảng 500 ns median;
  - cold `INPUT[]` build khoảng 2.8 µs median;
  - mocked `_emit` gồm clock và compatibility normalization khoảng 1.4 µs
    median;
  - counter aggregation khoảng 600 ns median.

Các số trên chỉ là baseline định hướng, không phải acceptance packet Windows.
Không được suy ra “game latency giảm” hoặc “sender đã đạt trần tuyệt đối” từ
chúng.

Mọi performance report phải tách riêng:

- `send_duration_pure_us`;
- `bookkeeping_us`;
- `dispatch_lateness_us`;
- `visible_lateness_us`;
- process và dispatch-thread CPU time;
- game-observed/audio-loopback onset error nếu claim liên quan accuracy
  end-to-end.

## 4. Issue register và quyết định ban đầu

| ID | Vấn đề | Mức | Quyết định |
|---|---|---:|---|
| C0 | Calibration có race publish timestamp với `WM_INPUT` | Critical | Sửa bắt buộc trước freshness/TTL |
| H1 | Preplay adaptive-spin probe chạy sai thread/QoS context | High | Sửa bắt buộc, giữ direct-mode semantics |
| H2 | Mid-song reprobe block tám lần sleep liên tục | High | Sửa bắt buộc bằng cooperative state machine |
| H3 | Raw Input proxy bị diễn đạt quá rộng; cache không có freshness policy | High | Đổi evidence label; TTL phải qua decision gate |
| H4 | Sender proxy có thể bị dùng làm căn cứ tối ưu mù | High/process | Thêm acceptance/evidence gate, không đổi runtime |
| H5 | P2 docs có khả năng drift về hold margin/probe statistics | High/docs | Reconcile với code trong từng behavior PR |
| M1 | Post-play `gc.collect()` rationale lỗi thời trên 3.14t | Medium | Instrument trước, chưa đổi policy |
| M2 | `_emit()` normalize legacy result mỗi lần send | Medium/low | Benchmark-gated; strict internal contract |
| M3 | Full telemetry giữ heap tuyến tính tới hard cap | Medium | Thiết kế `summary` chỉ khi long-session evidence đủ |
| M4 | Prewarm budget theo entry, không theo payload/hotness | Medium/low | Instrument trước; precision default không đổi |
| L1 | Lead cache ghi `saved_at` nhưng không dùng; fingerprint chưa rõ | Low | Cross-mode experiment trước schema v3 |
| L2 | Timing profile không phải device/power policy | RFC | Không retrofit profile; chỉ ship axis mới khi có Pareto evidence |

### 4.1 Điều chỉnh so với review gốc

- TTL không mặc nhiên an toàn hơn cache cũ. Một stale margin 2,000 µs có thể
  bảo thủ hơn fallback 500 µs; vì vậy không được “expire → 500” mà chưa có
  decision table và evidence.
- `gc.collect()` return value là số object collected/uncollectable, không phải
  số byte QSBR/mimalloc đã trả; RSS phải đo riêng.
- HUD đã dùng bounded `ProgressCounters`, không phụ thuộc full telemetry
  records. `summary` mode không được biện minh như một yêu cầu HUD giả.
- Timer mode/priority không nhất thiết thuộc fingerprint của pure
  `SendInput` syscall EMA. Nếu lead cache được tách, pure-send EMA và residual
  scheduling bias phải có compatibility rules riêng.
- Tám sample maximum không được gọi tùy tiện là “p90”. Telemetry/docs phải dùng
  thuật ngữ `sample_max` trừ khi triển khai percentile estimator thực.

## 5. Dependency và thứ tự triển khai

| Thứ tự | Workstream | Phụ thuộc | Có được ship ngay? |
|---:|---|---|---|
| 0 | Baseline freeze + evidence contract + doc drift audit | Không | Có, docs/tests only |
| 1 | C0 calibration concurrency correctness | Phase 0 | Có, bắt buộc |
| 2 | H1 dispatch-context preplay probe | Phase 0 | Có, bắt buộc |
| 3 | H2 cooperative mid-song reprobe | Phase 2 | Có, bắt buộc |
| 4 | H3 calibration label/schema/freshness | Phase 1 | Label có; TTL cần gate |
| 5 | M1 GC instrumentation | Phase 0 | Có, instrumentation only |
| 6 | M2 strict sender-result contract | Phase 0 benchmark | Chỉ khi perf gate đạt |
| 7 | M3 telemetry detail levels | Phase 0 memory benchmark | Chỉ khi ROI gate đạt |
| 8 | M4 prewarm instrumentation/policy | Phase 0 | Instrument có; policy cần gate |
| 9 | L1 lead-cache compatibility | Phase 2 evidence | Chỉ khi cross-mode gate đạt |
| 10 | L2 runtime power policy RFC | Phases 2, 3, 8 | RFC trước; behavior cần Pareto gate |
| 11 | Integration acceptance packet | Các phase đã chọn | Bắt buộc trước merge/release |

Phase 1 và 2 có thể được phát triển độc lập nhưng phải là hai PR/commit riêng.
Phase 3 cần biết lifecycle/telemetry thực tế sau Phase 2. Các phase 6–10 không
được chặn các correctness phase 1–4.

## 6. Protocol bắt buộc cho AI executor

Trước mỗi phase:

1. Xác nhận branch/commit và `git status`; không đè thay đổi của người dùng.
2. Đọc `AGENTS.md`, `SECURITY.md` nếu chạm input/calibration, và hàng
   “Navigation Map” tương ứng.
3. Đọc lại các P2 owner docs:
   - [`architecture.md`](../architecture.md);
   - [`rt-dispatch-architecture.md`](../rt-dispatch-architecture.md);
   - [`timing-principles.md`](../timing-principles.md);
   - [`timing-profile-frame-model.md`](../timing-profile-frame-model.md).
4. Viết test tái hiện failure/contract trước production change.
5. Chỉ sửa các file được liệt kê cho phase; nếu cần mở rộng scope, dừng và ghi
   lý do trước khi sửa.
6. Chạy targeted test, Ruff/Pyright phù hợp, rồi mới chạy broader gate.
7. Nếu behavior đã document thay đổi, cập nhật P2 owner doc trong cùng logical
   change.
8. Ghi evidence và quyết định `GO`, `NO-GO` hoặc `DEFER`. `NO-GO` là kết quả
   hợp lệ; không hạ benchmark threshold để ép merge.

Không commit, push, sửa golden schedules, `perf-baselines/*`, existing plan
history, spec, interpreter pin hoặc security audit files nếu chưa được yêu cầu/
phê duyệt theo `AGENTS.md`.

## 7. Phase 0 — Baseline freeze và truth reconciliation

### Mục tiêu

Tạo packet có thể so sánh trước/sau và xác định chính xác docs nào đang lệch
code. Không đổi runtime behavior.

### Công việc

1. Lưu commit SHA, Windows build, CPU class, Python version/cache tag, trạng
   thái GIL, AC/battery, power plan và background-load profile.
2. Chạy 50 cold starts và 50 warm starts cho benchmark Windows có thật. Không
   trộn cold/warm distribution.
3. Chạy corpus:
   - short song để đánh giá first 10 notes;
   - long/repetitive song;
   - high shape-diversity song;
   - same-key/frame-boundary song;
   - telemetry off và full.
4. Audit actual formula/code cho:
   - frame-derived hold;
   - `min_hold_margin_us`;
   - calibration clamp/fallback;
   - preplay probe sample/formula;
   - mid-song reprobe sample/formula;
   - `visible_lateness_us` semantics.
5. Lập truth table `code → test → normative statement`. Sửa wording P2 chỉ khi
   code/test đã chứng minh; không dùng proposal cũ làm nguồn chân lý.

### Targeted gate

```powershell
uv run --env-file .env pytest tests/test_adaptive_spin.py tests/test_spin_reprobe.py tests/test_calibration.py tests/test_adaptive_lead.py tests/test_dispatch_fidelity_refactor.py tests/test_post_play_memory_hygiene.py tests/test_hud_onset_metric.py tests/test_core_boundary.py -q
uv run --env-file .env python scripts/audit_free_threaded_wheels.py
uv run --env-file .env python scripts/audit_security_mandates.py
```

### Exit criteria

- Có raw result tách từng metric, không chỉ một số “latency”.
- Có environment manifest không chứa serial/PII/local path không cần thiết.
- Mọi doc drift được gán owner phase; chưa có behavior change.

## 8. Phase 1 — Sửa race calibration trên free-threaded Python

### Vấn đề cần tái hiện

Injection thread hiện lấy timestamp sau `SendInput` return rồi publish
`last_send_time_ns`. `WM_INPUT` callback có thể chạy đồng thời và đọc `0` hoặc
timestamp của sample trước. Với 3.14t, không được dựa vào incidental
single-thread/GIL ordering.

### File dự kiến

- `src/sky_music/platform/win32/calibration.py`
- `tests/test_calibration.py`
- `tests/test_core_send_overhaul_invariants.py`
- P2 doc mô tả calibration evidence nếu đang claim ordering cũ

### Thiết kế bắt buộc

1. Tạo pending-sample state có sequence ID, expected event kind/scan code,
   completion timestamp, receive timestamp và completion event.
2. Injection thread phải publish sequence/expected event **trước** native
   `SendInput`.
3. Ngay sau native return, injection thread lấy completion timestamp trước
   bookkeeping rồi publish nó dưới explicit synchronization.
4. `WM_INPUT` callback chỉ:
   - validate raw input/event kind;
   - lấy receive timestamp càng sớm càng tốt;
   - gắn timestamp vào đúng sequence;
   - signal event.
   Callback không tự tính/append latency dựa trên shared timestamp có thể cũ.
5. Injection thread chỉ finalize sample khi đã có cả hai timestamp cho cùng
   sequence.
6. Nếu receipt xảy ra trước native return, record một reorder counter và dùng
   semantics được test rõ (`max(0, receive - completion)` cho non-negative
   delivery proxy, hoặc signed diagnostic field riêng). Không được âm thầm
   ghép receipt với sample trước.
7. Timeout, duplicate/mismatched Raw Input và late callback phải bị bỏ có chủ
   đích; không poison sample kế tiếp.
8. Không thay margin formula, clamp hoặc cache TTL trong phase này.

### Test bắt buộc

- Callback đến sau completion publish.
- Callback đến trước completion publish.
- Callback đến trong native-call window.
- Late callback của sequence N sau khi N+1 đã bắt đầu.
- Wrong scan code/event kind.
- Timeout không để lại pending state.
- Concurrent stress dưới free-threaded runtime.
- Timestamp được lấy trước error formatting, cache write hoặc telemetry.

### Acceptance

- Không sample nào dùng timestamp `0` hoặc timestamp của sequence khác.
- Sample ordering deterministic dưới controlled fake clock/callback.
- Existing margin formula output không đổi với cùng valid sample list.
- P0 `SendInput`-only và platform isolation audit green.

### Rollback trigger

Rollback nếu state machine làm mất valid samples đáng kể, callback path thêm
blocking/I/O, hoặc cần đưa Win32 state ra ngoài `platform/`.

## 9. Phase 2 — Đưa preplay adaptive probe vào đúng dispatch context

### Mục tiêu

Threaded playback phải probe timer wake error trên chính dispatch thread, bên
trong timer và priority scopes thực tế, trước final epoch rebase. Direct/
non-threaded execution vẫn phải có pre-anchor probe hợp lệ.

### File dự kiến

- `src/sky_music/orchestration/engine.py`
- `src/sky_music/orchestration/playback_supervisor.py`
- `src/sky_music/orchestration/core/loop.py` nếu cần primitive setter/protocol
- `src/sky_music/infrastructure/rt_priority.py` chỉ để expose outcome hiện có,
  không đổi ladder
- `tests/test_adaptive_spin.py`
- `tests/test_send_warmup_telemetry.py`
- tests về threaded dispatch, priority và epoch rebase
- `docs/rt-dispatch-architecture.md`

### Thiết kế bắt buộc

1. Giữ nguyên trong PR này:
   - 30 samples;
   - sleep target 2 ms;
   - `sample_max + 200 µs`;
   - current floor/cap;
   - telemetry schema;
   - kill switch.
2. Threaded path:
   - tạo dispatch thread;
   - enter timer scope;
   - enter `DispatchThreadPriorityScope`;
   - biết `thread_id`, timer mode, requested/acquired priority và
     power-throttling outcome;
   - chạy probe trên thread này;
   - apply threshold vào loop;
   - rebase final epoch;
   - gọi `run()`.
3. Direct path phải probe trong execution context hiện tại và rebase epoch sau
   probe. Không được bỏ adaptation khỏi test/direct mode.
4. Probe implementation vẫn qua injected wait/sleeper abstraction. Không
   import Win32/wall clock vào core.
5. Nếu priority acquire fail, vẫn probe dưới fallback context thực và record
   outcome; không giả vờ MMCSS đã thành công.
6. Probe exception/degradation phải fallback về configured threshold như hiện
   tại, không làm playback fail nếu policy hiện tại không fail.
7. First note của short song phải nhận threshold mới; không cần chờ mid-song
   reprobe.

### Test bắt buộc

- Probe thread ID bằng dispatch thread ID, khác caller ID trong threaded mode.
- Probe xảy ra sau priority/timer scope enter và trước epoch anchor.
- Epoch không chứa 30 × 2 ms preflight delay.
- Direct mode giữ ordering và threshold.
- Priority success, HIGHEST fallback và full acquire failure.
- EcoQoS opt-out success/failure được ghi đúng, không làm đổi formula.
- Kill switch bỏ probe nhưng vẫn anchor đúng.
- First 10 notes/short-song regression.

### Windows benchmark

Trên AC và battery, cold CPU và background CPU load:

- 50 caller-thread probes (baseline);
- 50 dispatch-context probes;
- `sample_max`, chosen threshold, first-10-note
  `visible_lateness_us` p99/max;
- thread/QoS outcome;
- CPU cost của preflight.

### Acceptance

- Structural/thread-order tests bắt buộc green.
- Không regress pure `SendInput` p99/max.
- Nếu Windows A/B cho thấy threshold distribution không khác đáng kể, change
  vẫn có thể ship như correctness-of-measurement, nhưng không được claim
  accuracy improvement.

## 10. Phase 3 — Cooperative mid-song reprobe

### Mục tiêu

Giữ tám samples và threshold formula nhưng command/focus có thể được service
giữa mỗi sample.

### File dự kiến

- `src/sky_music/orchestration/core/loop.py`
- `tests/test_spin_reprobe.py`
- `tests/test_adaptive_spin.py`
- focus/pause/threaded command interruption tests
- `docs/rt-dispatch-architecture.md`

### Thiết kế bắt buộc

1. Thay block tám sleep bằng bounded probe state:
   - `sample_count`;
   - `sample_max_error_us`;
   - current attempt ID;
   - next eligibility/cancel state.
2. Thu tối đa một sample trong mỗi outer wait iteration.
3. Sau mỗi sample, quay lại service command/focus path trước sample tiếp theo.
4. Nếu pause, stop, seek/focus transition hoặc relevant command interrupt:
   discard toàn bộ partial attempt và thử lại ở eligible gap sau.
5. Mỗi sample chỉ được chạy khi remaining deadline vẫn vượt safety gap hiện
   tại. Re-check deadline trước từng sample, không chỉ đầu attempt.
6. Chỉ commit candidate sau đủ tám samples.
7. Giữ `sample_max + 200`, floor/cap/hysteresis và telemetry list hiện tại.
8. Đổi comment/docs `p90` thành `sample maximum` nếu implementation không dùng
   percentile estimator thực.

### Test bắt buộc

- Inject command/focus event tại sample index 0–7.
- Ack latency không bị block bởi bảy sample còn lại.
- Không có note-on sau focus loss.
- Partial attempt bị discard, không trộn sample giữa attempts.
- Không-interrupt candidate bằng implementation cũ với cùng fake samples.
- Deadline trở nên gần giữa attempt thì probe yield/cancel.
- Pause/resume/stop lifecycle và reprobe telemetry.

### Acceptance

- Worst-case command acknowledgement bị giới hạn xấp xỉ một sample sleep cộng
  bounded overhead, không phải toàn bộ tám samples.
- Không regress note deadline sau idle gap.
- Không giảm sample count hoặc đổi threshold formula trong cùng PR.

## 11. Phase 4 — Calibration evidence label, schema và freshness

Phase này tách thành hai logical changes; không merge label-only với TTL
behavior.

### 11.1 Phase 4A — Honest evidence label

Đổi terminology thành `injected_raw_input_delivery_proxy` hoặc một tên tương
đương được dùng nhất quán trong:

- calibration result/cache metadata;
- doctor message;
- runtime options/telemetry description;
- normative docs.

UI phải nói rõ calibration:

- dùng app-owned Win32 window;
- inject bằng `SendInput`;
- quan sát `WM_INPUT`;
- không đo Sky process polling, render frame hoặc audio onset.

Nếu đổi serialized field/schema, phải version rõ; không silently reinterpret
cache v1.

### 11.2 Phase 4B — Freshness policy, chỉ sau evidence

Trước khi chọn TTL:

1. Calibrate trước/sau reboot.
2. Calibrate sau Windows update nếu có môi trường.
3. Đổi AC ↔ battery và power plan.
4. Đo dưới background CPU/DPC load.
5. Nếu có human-approved black-box audio loopback, kiểm tra correlation; không
   hook/process inspection.

Schema candidate phải có:

- explicit schema version;
- timezone-aware UTC `sampled_at`;
- evidence kind;
- sample count và source formula version;
- optional non-PII context đủ để giải thích invalidation.

Loader phải test:

- missing/malformed timestamp;
- naive vs timezone-aware timestamp;
- future timestamp/clock skew;
- exact TTL boundary;
- unsupported version;
- atomic validate-then-apply.

### Freshness decision table phải được chốt trước code

| State | Câu hỏi phải trả lời |
|---|---|
| Fresh valid | Dùng calibrated margin trực tiếp hay clamp hiện tại? |
| Stale but syntactically valid | Prompt recalibration, conservative fallback hay ignore? |
| Stale value > default | Có được giảm về default 500 µs không? |
| Stale value < default | Có được dùng để giảm safety margin không? |
| Missing timestamp v1 | Migrate one-time, treat stale hay reject? |
| Future timestamp | Clock-skew tolerance bao nhiêu và surface thế nào? |

Default safety rule nếu chưa có quyết định: cache cũ tiếp tục behavior hiện tại;
chỉ thêm diagnostic. Không ship arbitrary TTL.

### Acceptance

- Evidence scope trung thực ở UI/docs/telemetry.
- TTL được derive từ repeated measurements, không phải số chọn cảm tính.
- Stale cache không âm thầm làm giảm safety.
- Cache parse fail vẫn deterministic và không crash playback.

## 12. Phase 5 — Modernize và instrument GC lifecycle

### Mục tiêu

Sửa rationale từ pymalloc/GIL-era sang 3.14t mimalloc/QSBR và thu bằng chứng
trước khi cân nhắc conditional collection.

### Thiết kế

1. Giữ pre-play/post-play `gc.collect()` behavior ở PR đầu.
2. Record cho mỗi collect:
   - duration;
   - `gc.collect()` return count;
   - phase (`pre_play`/`post_play`);
   - schedule batch count;
   - telemetry record count;
   - input-array cache entries/slots vừa clear.
3. RSS/working set được đo bởi benchmark/infrastructure, không giả định từ
   return count.
4. Không gọi wall clock trực tiếp từ `orchestration/core/`; dùng injected clock
   hoặc infrastructure lifecycle outcome.
5. Instrumentation phải bounded và không log local paths.
6. Sửa comment nói rõ 3.14t có deferred reference counting/QSBR và mimalloc;
   RSS có thể không giảm ngay dù objects đã reclaim.

### Benchmark

- 100 short songs liên tiếp.
- 20 large songs.
- RSS: trước clear, sau clear, sau collect, sau 1 giây idle.
- collect duration p50/p95/p99/max.
- time-to-next-picker-frame.
- return count distribution.

### Gate cho behavior PR riêng

Chỉ thử threshold/idle/conditional collect nếu current collect tạo hitch có ý
nghĩa và benchmark branch chứng minh:

- memory hygiene không regress;
- next-session first frame tốt hơn;
- deferred memory không tăng không giới hạn;
- thread-exit/QSBR cleanup vẫn hoàn tất.

Không đạt gate: giữ collect-every-song, chỉ ship comment/instrumentation.

## 13. Phase 6 — Strict trusted sender result contract

### Mục tiêu

Loại compatibility normalization khỏi mỗi `_emit()` chỉ khi có lợi ích đo được.

### Thiết kế candidate

1. Production trusted sender phải trả chính xác `PlatformSendResult`.
2. Chuyển toàn bộ test doubles đang trả `None`, `int` hoặc tuple sang
   type-compatible result.
3. Nếu public/downstream legacy callable còn được support, wrap/normalize đúng
   một lần lúc backend construction; hot path chỉ thấy strict callable.
4. Boundary validation phải fail rõ với invalid type; không chuyển silent
   failure thành success.
5. Không đổi partial-send retry, release policy, error code hoặc completion
   timestamp placement.

### Benchmark gate

Dùng các scaffold hiện có:

```powershell
uv run --env-file .env pytest tests/bench_backend_result_normalization.py tests/bench_dispatch_send_pedantic.py -m slow --benchmark-only --benchmark-disable-gc -q
```

Chỉ merge nếu:

- isolated `_emit` cải thiện ít nhất 5% qua nhiều round;
- dispatch-send benchmark không regress p99/max;
- end-to-end cho thấy ít nhất 1% CPU improvement **hoặc** tail distribution
  tốt hơn có ý nghĩa;
- kết quả giữ được trên Python 3.14t, không chỉ mocked CPython path.

Không đạt gate: đóng candidate `NO-GO`, giữ compatibility shim.

## 14. Phase 7 — Telemetry detail levels

### Mục tiêu

Cho long-session diagnostics dùng bounded aggregate khi không cần từng event,
trong khi `full` vẫn tương thích CSV hiện tại.

### Contract candidate

- `off`: behavior hiện tại khi telemetry disabled.
- `summary`: online bounded counters/histograms, không retain per-event
  `TelemetryRecord`.
- `full`: schema, ordering, raw rows và save behavior hiện tại bit-for-bit.

### Quyết định thiết kế bắt buộc

1. Xác định metric nào exact (`count`, failures, min/max, sum/mean).
2. Nếu giữ p50/p95/p99, định nghĩa fixed-memory histogram/reservoir và error
   bounds; không gọi approximate quantile là exact.
3. Không thêm dependency mới nếu standard structures đủ.
4. HUD vẫn dùng `ProgressCounters`; không route HUD qua summary logger.
5. Cap/truncation và `retain_records_after_save` của full mode không đổi.
6. Save/export không chạy I/O trên dispatch thread.

### Benchmark gate

Synthetic 100,000 dispatches dưới 3.14t, so sánh `off/summary/full`:

- events/second;
- process và dispatch-thread CPU time;
- `tracemalloc` peak;
- peak working set;
- teardown/save duration;
- quantile error nếu summary approximate.

Chỉ ship nếu summary có use case rõ, fixed memory và giảm đáng kể heap/CPU so
với full. Default mode không đổi trong first PR.

## 15. Phase 8 — Prewarm observability và optional budget policy

### Phase 8A — Instrument trước

Record ngoài timed send path:

- unique down/up shape count;
- total `INPUT` slots và approximate payload bytes;
- frequency distribution;
- prewarm duration;
- cache misses trong timed playback;
- first-hit lazy-build duration;
- peak RSS;
- clear-cache outcome.

Không đổi exact-shape prewarm default.

### Phase 8B — Policy candidate, chỉ sau corpus benchmark

Corpora:

1. Ít shape, lặp nhiều.
2. Hàng nghìn unique shape.
3. Mixed hot/cold shapes.

Nếu Phase 8A cho thấy cost thực:

- precision mode giữ full exact-shape prewarm;
- bounded/efficiency mode dùng budget theo total `INPUT` slots/bytes, không chỉ
  entry count;
- singleton down/up luôn được ưu tiên;
- repeated/hot shapes được sort trước bằng schedule frequency;
- rare misses dùng lazy cache hiện có;
- không đổi ctypes representation trong cùng PR.

### Acceptance

- Precision default bit-for-bit và không có timed cache miss mới.
- Budgeted mode có cap memory/preflight rõ.
- First-hit p99/max và missed-deadline distribution không regress vượt
  threshold đã ghi trước benchmark.
- Không ship policy nếu corpus thông thường không cho ROI.

## 16. Phase 9 — Lead-cache TTL/fingerprint experiment

### Mục tiêu

Xác định stale lead state có thực sự tệ hơn cold estimator trong first 10 notes
trước khi tăng schema complexity.

### Experiment

Tạo cache ở từng context rồi cross-load:

- Python ABI/cache tag và GIL state;
- backend implementation version;
- high-resolution timer vs fallback;
- priority requested/acquired outcomes;
- power-throttling outcome;
- AC/battery/power mode.

Đo separately:

- pure send-duration EMA error;
- residual scheduling bias;
- first 10 down/up completion errors;
- p99/max và sample count tới hội tụ;
- cold estimator baseline.

### Schema v3 candidate

Chỉ khi cross-mode p99 xấu hơn cold start rõ rệt:

1. `saved_at` UTC + finite TTL dựa trên evidence.
2. PII-free fingerprint.
3. Tách compatibility:
   - pure-send EMA chỉ invalidated bởi runtime/backend factors đã chứng minh;
   - scheduling residual invalidated bởi timer/priority/power context nếu đã
     chứng minh.
4. Parse candidate sớm nhưng chỉ activate context-dependent portion sau khi
   dispatch thread đã biết acquired runtime outcomes.
5. Mismatch → cold estimator; không cố transform cache cũ.
6. Không đổi EMA alpha, seed count và 2 ms clamp trong migration PR.

Không đạt experiment gate: đọc/validate `saved_at` chỉ để diagnostic hoặc giữ
schema hiện tại; không thêm fingerprint suy đoán.

## 17. Phase 10 — Runtime power policy RFC

Timing profiles hiện là musical/safety profiles. Không đổi semantics của
`local_precise`, `balanced`, `audience_safe` để biến chúng thành hardware
profiles.

### RFC candidate

Axis độc lập, tên ví dụ:

- `precision`;
- `standard`;
- `efficiency`.

RFC phải quyết định toàn bộ các knob liên quan, không chỉ spin cap:

- adaptive spin cap/floor;
- idle warmup budget;
- reprobe cadence/enablement;
- prewarm payload budget;
- requested priority ladder;
- EcoQoS opt-out/allow policy;
- telemetry default/detail;
- AC/battery automatic selection có opt-out hay không.

Power policy **không được** thay:

- `min_hold_frames`;
- same-key/conflict policy;
- release retry/cleanup;
- chord batching;
- focus safety.

### Compatibility

- Khi config không có axis mới, behavior phải bit-for-bit như hiện tại.
- Config parsing/round-trip và CLI overrides phải giữ compatibility.
- Không tự động dùng `TIME_CRITICAL`.
- Không gộp RFC/config migration với telemetry hoặc prewarm implementation.

### Pareto gate

Chạy cross-product timing profile × power policy trên AC và battery:

- process/dispatch CPU time;
- wakeups/second;
- p50/p99/max visible lateness;
- drops/hold-floor violations (phải bằng 0);
- preflight time;
- peak RSS;
- acquired priority/power outcome.

Chỉ ship những policy nằm trên Pareto frontier và có UX/default rõ. Nếu
adaptive spin hiện tại đã đủ tốt, giữ config đơn giản và đóng RFC `NO-GO`.

## 18. Normative documentation matrix

| Behavior/evidence | Owner doc | Update cùng phase |
|---|---|---:|
| Sender completion vs game/audio truth | `timing-principles.md` | 0/4A |
| Probe thread/order/epoch rebase | `rt-dispatch-architecture.md` | 2 |
| Cooperative reprobe + `sample_max` wording | `rt-dispatch-architecture.md` | 3 |
| Frame hold và `min_hold_margin_us` formula | `timing-profile-frame-model.md`, `timing-principles.md`, `architecture.md` | 0 audit; owner behavior PR |
| Calibration evidence/cache semantics | `architecture.md`, relevant timing doc | 4 |
| Priority/power axis | `rt-dispatch-architecture.md`, timing profile doc | 10 nếu ship |
| GC rationale | nearby production comment; architecture only nếu contract đổi | 5 |
| Telemetry detail modes | `rt-dispatch-architecture.md` nếu runtime contract đổi | 7 |

Không tạo một docs-only “future behavior” rồi để code theo sau. P2 phải mô tả
behavior đã ship hoặc đánh dấu rõ proposal.

## 19. Test và validation matrix

### Targeted tests theo surface

| Surface | Test chính |
|---|---|
| Calibration race/cache | `tests/test_calibration.py`, `tests/test_core_send_overhaul_invariants.py` |
| Adaptive preplay | `tests/test_adaptive_spin.py`, `tests/test_send_warmup_telemetry.py`, threaded/priority/epoch tests |
| Mid-song reprobe | `tests/test_spin_reprobe.py`, focus/pause/command interruption tests |
| Lead cache | `tests/test_adaptive_lead.py` |
| Sender contract | `tests/test_dispatch_fidelity_refactor.py`, normalization/send benchmarks |
| Telemetry | telemetry CSV/summary/golden-schema tests, `tests/test_hud_onset_metric.py` |
| Prewarm/memory | `tests/test_inputs_prewarm.py`, `tests/test_post_play_memory_hygiene.py`, resource-wiring tests |
| Architecture | `tests/test_core_boundary.py`, import-boundary/security invariants |

### Gate cho từng PR

```powershell
uv run ruff check .
uv run pyright
uv run pytest <targeted-tests>
```

### Gate pre-merge cho toàn chương trình

```powershell
uv run ruff check .
uv run pyright
uv run pytest
uv run --env-file .env python scripts/audit_free_threaded_wheels.py
uv run --env-file .env python scripts/audit_security_mandates.py
```

Windows live benchmark là bắt buộc cho performance claim. Nếu môi trường
không chạy được Windows 11/3.14t benchmark, AI phải báo `AWAITING EVIDENCE`,
không tự gán speed-up và không merge behavior candidate phụ thuộc gate.

## 20. Benchmark protocol chung

1. Warm-up benchmark harness riêng, không trộn với cold-start samples.
2. Ít nhất 50 cold và 50 warm runs cho startup/timing context.
3. Báo median, p95, p99, max, confidence/noise information và raw sample count.
4. Chạy AC/battery, idle/background CPU load; ghi priority/timer outcomes.
5. Randomize/interleave A/B khi thermal drift có thể ảnh hưởng.
6. Không chạy telemetry full rồi so với baseline telemetry off.
7. Không cộng sender duration, bookkeeping và game/audio onset thành một metric.
8. Không sửa committed `perf-baselines/*` nếu chưa có phê duyệt. Acceptance
   artifacts có thể để ở untracked/temp location hoặc đính kèm CI/PR.
9. Kết quả game-observed chỉ dùng black-box/human-approved capture; P0
   prohibitions vẫn áp dụng tuyệt đối.

## 21. Risk và rollback

| Risk | Mitigation | Rollback trigger |
|---|---|---|
| Probe moved nhưng epoch/order sai | Controlled clock + ordering test | First-note anchor chứa probe delay |
| Calibration state machine mất callback | Sequence/timeout stress tests | Sample loss/error rate tăng đáng kể |
| Cooperative reprobe trộn attempts | Attempt ID + cancel tests | Candidate khác baseline khi không interrupt |
| TTL làm giảm safety margin | Decision table + conservative gate | Stale handling giảm margin không được chứng minh |
| Strict result contract phá mocks/downstream | Normalize once at boundary | Invalid result bị coi success hoặc release regress |
| Summary telemetry sai quantile | Document error bounds + differential tests | Summary ngoài declared error |
| Budgeted prewarm tạo tail miss | Precision default + corpus benchmark | Timed miss/p99 regress |
| Conditional GC giữ memory | Repeated-song RSS/QSBR test | RSS/deferred state tăng không giới hạn |
| Power axis phình config | RFC/Pareto gate | Không có user-visible tradeoff rõ |

Mỗi behavior phase phải rollback được bằng một commit riêng. Không tạo mega-PR
gồm probe, calibration, telemetry, cache và GC.

## 22. Suggested commit/PR sequence

1. `test(windows): reproduce calibration timestamp race`
2. `fix(windows): correlate calibration samples safely`
3. `test(scheduler): pin adaptive probe execution context`
4. `fix(scheduler): probe wake error on dispatch thread`
5. `test(scheduler): cover reprobe interruption at every sample`
6. `fix(scheduler): make adaptive reprobe cooperative`
7. `docs(timing): clarify raw-input proxy and sample-max semantics`
8. `chore(runtime): instrument free-threaded gc lifecycle`
9. `refactor(windows): enforce trusted send result contract` — chỉ khi gate đạt
10. `feat(telemetry): add bounded summary detail mode` — chỉ khi gate đạt
11. `chore(windows): measure prewarm payload and misses`
12. `feat(runtime): add budgeted prewarm policy` — chỉ khi gate đạt
13. `feat(scheduler): version context-compatible lead cache` — chỉ khi gate đạt
14. `feat(runtime): add independent power policy` — chỉ sau RFC/Pareto gate

Tên có thể điều chỉnh theo diff thực tế, nhưng một logical behavior change trên
mỗi commit/PR là bắt buộc.

## 23. Stop/escalation conditions

AI phải dừng và xin hướng dẫn nếu:

- solution cần bất kỳ game inspection/hook/process access nào;
- solution đề xuất input mechanism khác `SendInput`;
- cần sửa security audit/baseline, updater, spec strategy, interpreter pin,
  dependency core hoặc protected immutable artifact;
- benchmark contradicts proposed optimization;
- normative docs và actual code không thể reconcile mà không đổi musical
  semantics;
- direct mode và threaded mode không thể giữ cùng public contract;
- strict validation hoặc release safety bị coi là “overhead cần bỏ”;
- Windows live evidence là điều kiện acceptance nhưng không có môi trường chạy.

## 24. Definition of Done

### Bắt buộc

- [ ] C0 calibration race có deterministic regression test và đã được sửa.
- [ ] H1 probe chạy đúng dispatch context trong threaded mode.
- [ ] Direct mode vẫn probe/rebase đúng.
- [ ] H2 reprobe service command/focus giữa samples.
- [ ] Calibration được gọi đúng là injected Raw Input delivery proxy.
- [ ] `sample_max`/percentile terminology chính xác.
- [ ] P2 docs khớp code/test đã ship.
- [ ] Không regress one-batch chord, completion timestamp, partial-send/release
      safety.
- [ ] Core/platform import boundaries green.
- [ ] Ruff, Pyright, full pytest, free-threaded audit và security audit green.

### Có điều kiện

- [ ] TTL chỉ ship sau freshness decision/evidence.
- [ ] GC policy chỉ đổi sau instrumentation benchmark.
- [ ] Compatibility shim chỉ bỏ khi vượt perf gate.
- [ ] Summary telemetry chỉ ship khi fixed-memory ROI rõ.
- [ ] Budgeted prewarm chỉ ship khi corpus chứng minh lợi ích.
- [ ] Lead-cache v3 chỉ ship khi stale cross-mode tệ hơn cold.
- [ ] Runtime power policy chỉ ship khi có Pareto frontier và backward-compatible
      default.

### Acceptance packet cuối

- Commit SHA và environment manifest.
- Before/after targeted + full validation logs.
- Security/free-threaded audit results.
- Raw benchmark data và summary tách cold/warm, sender/bookkeeping/game proxy.
- Thread/QoS/timer outcome cho adaptive probe.
- Calibration concurrency test trace.
- Command/focus interruption latency cho reprobe.
- Memory/GC/prewarm/telemetry evidence cho phase đã ship.
- Danh sách candidate `GO`, `NO-GO`, `DEFER`, không che giấu negative result.
- Diff normative docs tương ứng với behavior.

## 25. Evidence execution record — 2026-07-28

The remaining gated work was measured with the isolated harness
[`scripts/bench_remaining_plan.py`](../../scripts/bench_remaining_plan.py). The
harness uses the current telemetry/estimator contracts and local prototypes; it
does not change production defaults, input technology, lead-cache schema or
power policy.

### Environment and reproducibility

- Windows host, CPython `3.14.3` free-threading build, runtime GIL disabled.
- Repro command:

  ```powershell
  uv run --env-file .env python scripts/bench_remaining_plan.py
  ```

- Phase 7 was repeated three times per mode and reported median values. The
  external process monitor measured approximately 24.4–24.5 MB peak working
  set for each short isolated run; this monitor did not provide a useful
  mode-to-mode delta, so `tracemalloc` is the differential memory evidence.

### Phase 6 — M2 strict sender-result contract

Command:

```powershell
uv run --env-file .env pytest tests/bench_backend_result_normalization.py tests/bench_dispatch_send_pedantic.py -m slow --benchmark-only --benchmark-disable-gc -q
```

Result: `3 passed`. The observed baseline remained approximately 500 ns median
for cached lookup, 3.2 µs for cold array build and 1.5 µs for the compatibility
shim. The scaffold has no strict-candidate arm and no live end-to-end CPU/tail
measurement, so the ≥5% isolated and ≥1% end-to-end gate cannot be established.

Decision: **NO-GO / DEFER**. Keep the compatibility shim.

### Phase 7 — M3 telemetry detail levels

Synthetic `100,000` dispatches, three-run median:

| Mode | Wall time | Process CPU | `tracemalloc` peak | Retained records |
|---|---:|---:|---:|---:|
| `off` | 407,612 µs | 406,250 µs | 3,683 B | 0 |
| bounded summary prototype | 232,000 µs | 218,750 µs | 1,752 B | 0 |
| `full` | 1,444,660 µs | 1,421,875 µs | 46,555,602 B | 100,000 |

The prototype demonstrates fixed-memory and lower CPU cost versus `full`, but
the current product has no user-facing detail-level contract, no declared
quantile error bounds, and the HUD already uses bounded `ProgressCounters`
without full telemetry. The benchmark therefore proves prototype mechanics,
not sufficient product ROI or a production-compatible schema.

Decision: **DEFER** M3 behavior. Do not add a summary mode or change the
default until the use case, exact metrics/error bounds and save/export contract
are specified. The current full/off behavior remains unchanged.

### Phase 8B — M4 budgeted prewarm policy

The existing exact-shape path was measured against three corpora. The policy
column below is an illustrative `2,048` total-`INPUT`-slot budget only; it is
not a proposed default threshold.

| Corpus | Events | Unique shapes | Precision slots | Prewarm | Budgeted slots | Budgeted event misses |
|---|---:|---:|---:|---:|---:|---:|
| few repeated | 100,000 | 4 | 6 | 1,527 µs | 2,048 | 0 |
| thousands unique | 10,000 | 10,000 | 10,000 | 5,590,952 µs | 2,048 | 15,904 |
| mixed hot/cold | 100,000 | 8,004 | 8,006 | 3,142,535 µs | 2,048 | 57,910 |

The budget saves payload slots only for high-diversity corpora and produces a
large lazy-miss surface. No timed-send p99/max or missed-deadline evidence was
collected for a production budget candidate, and the threshold is not derived
from a representative song corpus.

Decision: **DEFER** M4 policy. Keep Phase 8A diagnostics and exact-shape
precision default; do not ship a budgeted mode.

### Phase 9 — L1 lead-cache cross-mode experiment

The current estimator state was seeded in an `800/400 µs` context, exported and
cross-loaded into synthetic contexts. First-ten-note absolute completion-error
proxies were compared with a cold estimator:

| Context | Stale p99/max | Cold p99/max | Stale worse? |
|---|---:|---:|---|
| matching AC/high priority | 0/0 µs | 800/800 µs | No |
| battery/fallback priority | 800/800 µs | 1,600/1,600 µs | No |
| timer fallback | 400/400 µs | 1,200/1,200 µs | No |
| low-latency power state | 600/600 µs | 200/200 µs | **Yes** |

The result is mixed: stale state is beneficial in several synthetic transitions
but is 3× worse at p99 in the low-latency transition. It is not a Windows live
measurement and does not include real timer/priority/power outcomes or the
required convergence sample distribution.

Decision: **DEFER** schema v3/fingerprint activation. Keep schema v2 and use
diagnostic evidence only; do not add guessed invalidation rules.

### Phase 10 — L2 runtime power policy

Host observation at measurement time:

- active Windows plan: `Balanced`;
- `GetSystemPowerStatus`: `ACLineStatus=1`, `BatteryFlag=1`, battery level 100%;
- a live battery run was not available without changing physical/system power
  state, which was intentionally not performed.

No AC/battery cross-product with visible lateness, wakeups, RSS, priority/power
outcomes and missed-deadline distribution can therefore be claimed. No timing
profile semantics or power behavior was changed.

Decision: **AWAITING EVIDENCE / DEFER** the RFC behavior candidate. A future
run must collect the required AC and battery distributions before any policy
axis or automatic selection is implemented.

### Remaining-phase decision summary

| Candidate | Evidence status | Production action |
|---|---|---|
| M2 strict sender result | insufficient candidate/E2E gate | retain shim |
| M3 telemetry summary | prototype positive, product contract incomplete | defer behavior |
| M4 budgeted prewarm | high-diversity memory benefit but large miss surface | retain precision default |
| L1 lead-cache v3 | mixed synthetic cross-mode result | retain schema v2 |
| L2 power policy | AC-only observation; battery Pareto missing | defer RFC behavior |

No conditional candidate is promoted to `GO` by this packet. Phase 8A
observability remains the only optional instrumentation shipped from this
evidence pass.

## 26. Primary references

- [`AGENTS.md`](../../AGENTS.md)
- [`SECURITY.md`](../../SECURITY.md)
- [`docs/INDEX.md`](../INDEX.md)
- [`docs/architecture.md`](../architecture.md)
- [`docs/rt-dispatch-architecture.md`](../rt-dispatch-architecture.md)
- [`docs/timing-principles.md`](../timing-principles.md)
- [`docs/timing-profile-frame-model.md`](../timing-profile-frame-model.md)
- Microsoft `SendInput`:
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput>
- Microsoft high-resolution waitable timer:
  <https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw>
- Microsoft thread power throttling:
  <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadinformation>
- Microsoft MMCSS:
  <https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service>
- Python 3.14 free-threading:
  <https://docs.python.org/3.14/howto/free-threading-python.html>
- Python 3.14 `gc`:
  <https://docs.python.org/3.14/library/gc.html>

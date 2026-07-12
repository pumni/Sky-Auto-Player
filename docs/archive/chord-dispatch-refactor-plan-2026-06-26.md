# Kế hoạch refactor — Đường gửi phím / dispatch loop

> **Nguồn:** Dựa trên `docs/chord-dispatch-review-2026-06-26.md` (báo cáo đánh giá ngày 2026-06-26).
> **Đối tượng thực thi:** AI coding agent.
> **Mục tiêu:** Dọn dead code + rút ngắn "prologue" giữa spin-end và `SendInput` (giảm `visible_lateness` thật) + nâng type-safety, **không thay đổi hành vi timing/đồng thời của hợp âm**.
> **Trạng thái mã hiện tại:** branch `main` sạch. Mọi tham chiếu dòng dưới đây ứng với cây hiện tại — hãy đọc lại file trước mỗi sửa đổi để khớp chính xác.

> ⚠ **CẬP NHẬT 2026-07-13 — phần lớn Phase đã thực thi.** Bảng dưới ánh xạ Phase → commit / refactor.
>
> | Phase | Nội dung | Trạng thái | Commit / refactor |
> |---|---|---|---|
> | **A.1-A.4** | Rút ngắn prologue (lead path dedup + `getattr` spin + generator dedup) | DONE | `7cdb8fb perf(dispatch): trim hot-path allocations and tighten spin loop` + refactor 2026-07 (unified `_lead_for(kind, n_keys)` helper) |
> | **B.1-B.4** | Protocol `LeadEstimator` + default dedup | DONE | `LeadEstimator` Protocol ở `dispatch_loop.py:184`; engine truyền trực tiếp `_NullEstimator()` khi adaptive-lead tắt (refactor 2026-07) |
> | **C.1-C.3** | `statistics` dedup cho spin threshold | DONE | toàn bộ dùng `statistics.fmean`/`statistics.pstdev` |
> | **D.1** | Gỡ `deferred_by_us` trong `_execute_action` | DONE | tham số đã gỡ khỏi signature |
> | **D.2** | `command_event` trong `_process_wait_states` | NOT NEEDED | `_process_wait_states` không bao giờ được thêm `command_event` |
> | **E** | `Any` → `Protocol` cho command/focus/progress | DONE | không còn `: Any` nào trong `dispatch_loop.py` |
> | **F** | Gỡ global mirror "legacy" | DONE | `822c439 refactor(main): replace legacy globals with runtime state` |
> | **G.1** | ruff `target-version = "py314"` | DONE | `0b571eb refactor: tighten lint rules and simplify helpers` |
> | **G.2** | `requires-python` align | DONE | refactor 2026-07 (`pyproject.toml:14` → `>=3.14,<3.15`) |
>
> Refactor 2026-07 cover đồng thời A.5 (`_safe_debug_log` helper thay 5 khối `contextlib.suppress(Exception)` trong `release_all`) và A.7 (sort key scheduler bằng `_UP_BEFORE_DOWN_RANK` dict thay `a.kind == "down"` bool-as-int).
>
> Tài liệu này vẫn có giá trị lịch sử (mô tả quy trình + guardrail). Đường gửi hiện tại ở `docs/rt-dispatch-architecture.md`.

---

## 0. Bắt buộc đọc trước khi bắt đầu

### 0.1. Ràng buộc bất biến (P0 — KHÔNG được vi phạm)
- **SendInput ONLY.** Không thêm hook/injection/đọc bộ nhớ game. Không đổi cách gửi (vẫn một `SendInput` cho mỗi nhóm down/up).
- **Không đổi hành vi đồng thời hợp âm.** Một hợp âm = một `KeyAction` = một `key_down` = một `SendInput`. Không động vào `scheduler.build_key_actions` grouping, `_dispatch_down_batch`, `send_scan_code_batch*`.

### 0.2. KHÔNG ĐƯỢC ĐỘNG VÀO (đã đúng — xem §8.7 báo cáo)
- Vòng spin ns-based trong `wait_strategy.spin_until_us` (`infrastructure/wait_strategy.py:41-50`).
- Cơ chế `spin_threshold` adaptive (probe/reprobe mean+3σ) — chỉ được *dedup công thức*, không đổi giá trị.
- `RealtimeProcessScope` (GC pause + switch-interval), `DispatchThreadPriorityScope` (MMCSS/TIME_CRITICAL).
- Mô hình polyphonic linear trong `SendLatencyEstimator` (logic toán học giữ nguyên — chỉ được dedup cách gọi).
- Cột telemetry `deferred_by_us` trong `orchestration/telemetry.py` (xem cảnh báo §Phase D).

### 0.3. Lệnh kiểm thử (altitude table)
| Phạm vi thay đổi | Lệnh |
|---|---|
| Chỉ format/lint | `uv run ruff check .` |
| Chỉ type | `uv run pyright` |
| Chỉ test | `uv run pytest` |
| Trọn vẹn | `uv run ruff check . && uv run pyright && uv run pytest` |

### 0.4. Quy trình
1. Trước khi bắt đầu: chạy **trọn vẹn** để xác nhận baseline XANH. Ghi lại số test pass.
2. Mỗi Phase là **một commit độc lập**, tự kiểm thử xanh trước khi sang Phase kế.
3. Mỗi Phase độc lập — nếu một Phase rủi ro hơn dự kiến, có thể bỏ qua mà không chặn các Phase khác (trừ thứ tự ghi rõ).
4. Nếu một sửa đổi làm đỏ test không lường trước: **dừng, đọc lỗi, sửa gốc** — không retry mù, không nới lỏng assert để "cho xanh".

### 0.5. Thứ tự đề xuất (rủi ro tăng dần)
`A → C → D → B → E → G → F`. A–E là refactor lõi; F (gỡ global legacy) blast-radius lớn, để cuối; G (config) độc lập.

---

## Phase A — Rút ngắn prologue (giảm trễ thật, rủi ro thấp)

**Mục tiêu:** Loại công việc lặp trong khoảng "spin-end → SendInput". Hành vi timing **không đổi** (chỉ tính một lần thay vì nhiều lần các giá trị vốn bất biến trong khoảng đó).

### A.1. Tính `leads` một lần mỗi vòng, truyền vào `_drain_due`

**Lý do:** `get_current_leads()` đang được gọi 2 lần mỗi vòng lặp: trong `run()` (`dispatch_loop.py:904`) và lại trong `_drain_due` (`dispatch_loop.py:858`). Giữa hai lần này estimator **không bị cập nhật** (update chỉ xảy ra bên trong `_drain_due` *sau* khi đã đọc leads), nên giá trị giống hệt → an toàn để tính một lần và truyền xuống.

**LƯU Ý QUAN TRỌNG:**
- **GIỮ NGUYÊN** method `get_current_leads()` (signature + hành vi). Test `tests/test_runtime_dispatch.py:1179` gọi `loop.get_current_leads()` và mong tuple 2 phần tử.
- **KHÔNG** cố gỡ `lead_down` "vestigial" hay đổi nó thành cách tính khác — chỉ tránh gọi lặp.

**Sửa 1** — `_drain_due` nhận leads qua tham số. File `src/sky_music/orchestration/dispatch_loop.py`.

Hiện tại (≈ dòng 851-860):
```python
    def _drain_due(
        self,
        now_us: int,
        state: PlaybackState,
        first_action_executed: bool,
    ) -> tuple[ExecutionResult | None, ...]:
        results: list[ExecutionResult | None] = []
        lead_down, lead_up = self.get_current_leads()

        pending = self.coordinator.pop_due_pending(now_us, lead_up)
```
Đổi thành:
```python
    def _drain_due(
        self,
        now_us: int,
        state: PlaybackState,
        first_action_executed: bool,
        lead_down: int,
        lead_up: int,
    ) -> tuple[ExecutionResult | None, ...]:
        results: list[ExecutionResult | None] = []

        pending = self.coordinator.pop_due_pending(now_us, lead_up)
```

**Sửa 2** — `run()` truyền leads đã tính sẵn. Hiện tại (≈ dòng 903-929):
```python
            while not self.coordinator.is_finished():
                lead_down, lead_up = self.get_current_leads()
                deadline_us = self.coordinator.next_deadline_us(
                    lead_down, lead_up, lead_for_batch=self._down_lead_for_batch
                )
```
… (giữ nguyên phần wait) … rồi:
```python
                now_us = state.get_elapsed_us(self.clock)
                for result in self._drain_due(now_us, state, first_action_executed):
                    if result is not None:
                        first_action_executed = True
                    observe_result(result)
```
Đổi dòng cuối thành:
```python
                now_us = state.get_elapsed_us(self.clock)
                for result in self._drain_due(now_us, state, first_action_executed, lead_down, lead_up):
                    if result is not None:
                        first_action_executed = True
                    observe_result(result)
```
(`lead_down, lead_up` đã có sẵn từ đầu vòng — không cần đổi gì thêm. Biến vẫn còn nằm trong scope.)

### A.2. Bỏ `getattr` + nhánh chết ở cửa ngõ spin

**Lý do:** `HybridWaitStrategy` luôn có `spin_until_us` (Protocol bắt buộc; các test inject subclass đều override nó). Nhánh `else` là dead code, và `getattr` chuỗi chạy mỗi deadline.

File `dispatch_loop.py`, hiện tại (≈ dòng 771-782):
```python
            if remaining_us <= self.spin_threshold_us:
                self._wait_spin_start_us = elapsed_us
                spin_fn = getattr(self.wait_strategy, "spin_until_us", None)
                if spin_fn is not None:
                    spin_fn(target_system_us, self.clock)
                else:
                    while self.clock.now_us() < target_system_us:
                        pass
                if self.enable_reprobe:
                    after_elapsed = state.get_elapsed_us(self.clock)
                    self._record_overshoot(after_elapsed, target_elapsed_us)
                return None, last_runtime_poll_us, last_render_time_us, first_action_executed
```
Đổi thành:
```python
            if remaining_us <= self.spin_threshold_us:
                self._wait_spin_start_us = elapsed_us
                self.wait_strategy.spin_until_us(target_system_us, self.clock)
                if self.enable_reprobe:
                    after_elapsed = state.get_elapsed_us(self.clock)
                    self._record_overshoot(after_elapsed, target_elapsed_us)
                return None, last_runtime_poll_us, last_render_time_us, first_action_executed
```
**Xác minh trước khi sửa:** `grep -rn "wait_strategy" tests/` — đảm bảo mọi strategy inject trong test đều có `spin_until_us` (HybridWaitStrategy hoặc subclass override). Nếu có mock thiếu method, bổ sung method cho mock thay vì giữ nhánh chết.

### A.3. Bỏ generator thừa

File `dispatch_loop.py:518`, trong `_dispatch_pending_releases`:
```python
            generation_ids=tuple(g_id for g_id in gen_ids_list),
```
Đổi thành:
```python
            generation_ids=tuple(gen_ids_list),
```

### A.4. Kiểm thử Phase A
```
uv run pytest tests/test_adaptive_lead.py tests/test_runtime_dispatch.py tests/test_threaded_dispatch.py -q
uv run ruff check . && uv run pyright && uv run pytest
```
**Bất biến cần xanh:** `test_dispatch_completion_lands_on_schedule_with_warm_estimator`, `test_coordinator_per_batch_lead_scales_with_polyphony`, `test_down_lead_for_batch_*`.

**Commit:** `perf(dispatch): hoist lead computation out of drain, drop dead spin branch`

---

## Phase C — Dedup công thức spin-threshold bằng `statistics`

**Mục tiêu:** Thay mean/variance/stdev chép tay bằng `statistics.fmean`/`pstdev`. **Kết quả số phải tương đương** (population stdev = `pstdev`).

**KHÔNG** trừu tượng hóa thành helper dùng chung (hai chỗ có ngữ cảnh khác nhau — tránh over-abstraction). Sửa tại chỗ.

### C.1. `engine.py::_measure_spin_threshold`
Hiện tại (≈ dòng 396-412):
```python
    def _measure_spin_threshold(self, sleeper: Sleeper, *, prefix: str) -> int:
        import math as _math
        wake_errors: list[int] = []
        for _ in range(10):
            t0 = self.clock.now_us()
            sleeper.sleep(0.002)
            t1 = self.clock.now_us()
            wake_errors.append((t1 - t0) - 2_000)

        # Use mean + 3σ ...
        mean = sum(wake_errors) / len(wake_errors)
        variance = sum((e - mean) ** 2 for e in wake_errors) / len(wake_errors)
        stdev = _math.sqrt(variance)
        threshold = max(700, min(3_000, int(mean + 3 * stdev) + 100))
        self.effective_spin_threshold_us = threshold
```
Đổi thành (bỏ `import math as _math`; dùng statistics):
```python
    def _measure_spin_threshold(self, sleeper: Sleeper, *, prefix: str) -> int:
        wake_errors: list[int] = []
        for _ in range(10):
            t0 = self.clock.now_us()
            sleeper.sleep(0.002)
            t1 = self.clock.now_us()
            wake_errors.append((t1 - t0) - 2_000)

        # Use mean + 3σ rather than raw max ... (giữ nguyên comment hiện có)
        mean = statistics.fmean(wake_errors)
        stdev = statistics.pstdev(wake_errors)
        threshold = max(700, min(3_000, int(mean + 3 * stdev) + 100))
        self.effective_spin_threshold_us = threshold
```
Thêm ở đầu `engine.py` (khối import chuẩn): `import statistics`.

### C.2. `dispatch_loop.py::_recompute_spin_threshold_from_overshoot`
Hiện tại (≈ dòng 270-278):
```python
    def _recompute_spin_threshold_from_overshoot(self) -> int:
        if len(self._overshoot_samples) < 10:
            return self.spin_threshold_us
        samples = list(self._overshoot_samples)
        mean = sum(samples) / len(samples)
        variance = sum((x - mean) ** 2 for x in samples) / len(samples)
        stdev = math.sqrt(variance)
        new_threshold = max(700, min(3_000, int(mean + 3 * stdev) + 100))
        return new_threshold
```
Đổi thành:
```python
    def _recompute_spin_threshold_from_overshoot(self) -> int:
        if len(self._overshoot_samples) < 10:
            return self.spin_threshold_us
        mean = statistics.fmean(self._overshoot_samples)
        stdev = statistics.pstdev(self._overshoot_samples)
        return max(700, min(3_000, int(mean + 3 * stdev) + 100))
```
Đầu `dispatch_loop.py`: thêm `import statistics`. **Kiểm tra** `import math` ở đầu file (`dispatch_loop.py:3`) có còn dùng chỗ nào khác không (`grep -n "math\." src/sky_music/orchestration/dispatch_loop.py`) — nếu không còn thì gỡ `import math`.

### C.3. Kiểm thử
```
uv run pytest tests/ -k "spin or reprobe or threshold or calibrat" -q
uv run ruff check . && uv run pyright && uv run pytest
```
**Commit:** `refactor(timing): use statistics for spin-threshold mean/stdev`

---

## Phase D — Gỡ tham số chết

### D.1. `deferred_by_us` trong `_execute_action`

**Bối cảnh quan trọng (đọc kỹ):**
- `deferred_by_us` **được tính** ở `_dispatch_pending_releases` (`dispatch_loop.py:502`) và **được dùng** để chọn `runtime_outcome` (`:519`). → **GIỮ cả hai dòng này.**
- Nhưng nó **được truyền vào** `_execute_action` (`:520`) trong khi thân `_execute_action` **không hề dùng** → param chết.
- Cột telemetry `deferred_by_us` (`telemetry.py`) là **độc lập**: `telemetry.record()` đọc `getattr(result, "deferred_by_us", 0)` mà `ExecutionResult` không có field này → hiện luôn = 0. **KHÔNG đụng telemetry** (đó là vấn đề hành vi riêng, ngoài phạm vi refactor này). Tests `tests/test_runtime_dispatch.py:628`, `tests/test_measure_stutter.py:44` tham chiếu cột telemetry, **không** tham chiếu param của `_execute_action`.

**Sửa 1** — `dispatch_loop.py::_execute_action` signature (≈ dòng 325-335), bỏ dòng `deferred_by_us: int = 0,`:
```python
    def _execute_action(
        self,
        idx: int,
        action: KeyAction,
        state: PlaybackState,
        *,
        generation_ids: tuple[int, ...] = (),
        runtime_outcome: str = "sent",
        applied_lead_us: int = 0,
    ) -> ExecutionResult:
```

**Sửa 2** — nơi gọi trong `_dispatch_pending_releases` (≈ dòng 514-522). Hiện tại:
```python
        result = self._execute_action(
            best.source_action_index,
            action,
            state,
            generation_ids=tuple(g_id for g_id in gen_ids_list),
            runtime_outcome="deferred_release" if deferred_by_us > 0 else "sent",
            deferred_by_us=deferred_by_us,
            applied_lead_us=lead_up,
        )
```
Đổi thành (giữ `deferred_by_us` ở dòng `runtime_outcome`, bỏ dòng truyền param; lưu ý đã gộp A.3):
```python
        result = self._execute_action(
            best.source_action_index,
            action,
            state,
            generation_ids=tuple(gen_ids_list),
            runtime_outcome="deferred_release" if deferred_by_us > 0 else "sent",
            applied_lead_us=lead_up,
        )
```

**Sửa 3** — `engine.py` compat shim `_execute_action` (≈ dòng 619-638). Bỏ `deferred_by_us: int = 0,` khỏi signature và bỏ `deferred_by_us=deferred_by_us,` khỏi lời gọi forward:
```python
    def _execute_action(
        self,
        idx: int,
        action: KeyAction,
        state: PlaybackState,
        *,
        generation_ids: tuple[int, ...] = (),
        runtime_outcome: str = "sent",
        applied_lead_us: int = 0,
    ) -> ExecutionResult:
        return self._compat_dispatch_loop()._execute_action(
            idx=idx,
            action=action,
            state=state,
            generation_ids=generation_ids,
            runtime_outcome=runtime_outcome,
            applied_lead_us=applied_lead_us,
        )
```
**Xác minh:** `grep -rn "deferred_by_us" tests/ src/` — đảm bảo không còn ai gọi `_execute_action(..., deferred_by_us=...)`. (Các hit còn lại phải đều thuộc `telemetry.py` hoặc test telemetry — không đổi.)

### D.2. (TÙY CHỌN, có cổng kiểm tra) `command_event` trong `_process_wait_states` / `_service_control_state`

**Bối cảnh:** `command_event` **thực sự được dùng** tại `_wait_until_runtime_deadline` (truyền vào `wait_strategy.wait_until_us`, `dispatch_loop.py:803`) → **GIỮ ở đó**. Nhưng nó được luồn thừa qua `_service_control_state` (`:693, :725`) → `_process_wait_states` (`:617`) mà thân `_process_wait_states` không dùng.

**Cổng kiểm tra bắt buộc trước khi sửa:** đọc `tests/test_threaded_dispatch.py` quanh dòng 227 (`command_event: int | None,`). Nếu đó là chữ ký của một mock cho `wait_until_us` (KHÔNG phải cho `_process_wait_states`/`_service_control_state`), thì sửa D.2 an toàn. Nếu test gọi trực tiếp `_process_wait_states(..., command_event=...)`, **BỎ QUA D.2**.

Nếu cổng cho phép: bỏ tham số `command_event` khỏi signature `_process_wait_states` (`:617`) và `_service_control_state` (`:693`), và bỏ đối số `command_event=command_event` ở:
- lời gọi `_service_control_state(...)` trong `_wait_until_runtime_deadline` (`:757`, `:792`) — bỏ `command_event=command_event`.
- lời gọi `_process_wait_states(...)` trong `_service_control_state` (`:725`) — bỏ `command_event=command_event`.

### D.3. Kiểm thử
```
uv run pytest tests/test_runtime_dispatch.py tests/test_threaded_dispatch.py tests/test_measure_stutter.py tests/test_engine_refactor.py -q
uv run ruff check . && uv run pyright && uv run pytest
```
**Commit:** `refactor(dispatch): drop unused _execute_action/_process_wait_states params`

---

## Phase B — Dọn default + type cho estimator

**Mục tiêu:** Bỏ `Any` và nhánh re-default lòng vòng cho estimator.

### B.1. Khai báo Protocol `LeadEstimator`
Trong `dispatch_loop.py`, ngay trước `class _NullEstimator` (≈ dòng 175), thêm:
```python
from typing import Protocol  # nếu chưa import; gộp vào dòng `from typing import ...` hiện có


class LeadEstimator(Protocol):
    def get_lead_us(self, kind: str = "down", n_keys: int = 1) -> int: ...
    def update(self, kind: str, duration_us: int, n_keys: int = 1) -> None: ...
```
(`_NullEstimator` và `engine.SendLatencyEstimator` đều đã thỏa Protocol này — không cần kế thừa tường minh.)

### B.2. Đổi default + bỏ re-default
`dispatch_loop.py:213`:
```python
        estimator: Any = _NullEstimator(),
```
→
```python
        estimator: LeadEstimator | None = None,
```
`dispatch_loop.py:234`:
```python
        self.estimator = estimator if estimator is not None else _NullEstimator()
```
→ giữ nguyên (đây là nơi xử lý None tập trung — hợp lý). Annotation thuộc tính: `self.estimator: LeadEstimator = ...`.

### B.3. `engine.py` truyền estimator rõ ràng
`engine.py:390` hiện tại:
```python
            estimator=self.estimator if self.enable_adaptive_lead else None,
```
Giữ nguyên (None → `_NullEstimator` xử lý ở B.2). `self.estimator` là `SendLatencyEstimator` (thỏa `LeadEstimator`).

### B.4. Kiểm thử
```
uv run pyright && uv run pytest tests/test_adaptive_lead.py -q
uv run ruff check . && uv run pyright && uv run pytest
```
**Commit:** `refactor(dispatch): type estimator via LeadEstimator protocol`

---

## Phase E — Thay `Any` bằng Protocol cho command/focus/progress

**Mục tiêu:** `command_source`, `focus_signal`, `progress_sink` đang là `Any` khắp `dispatch_loop.py` dù đã có Protocol `CommandSource`/`FocusSignal`/`ProgressSink` trong `playback_supervisor.py:28-51`. Chỉ đổi annotation (không đổi logic).

**An toàn import:** `dispatch_loop.py` đã import runtime từ `playback_supervisor` (dòng 18). `playback_supervisor` chỉ import `dispatch_loop` dưới `TYPE_CHECKING` → **không tạo vòng lặp**. Có thể import runtime:
```python
from sky_music.orchestration.playback_supervisor import (
    CommandSource,
    FocusSignal,
    ProgressSink,
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SKIPPED,
)
```
Thay mọi annotation `command_source: Any` → `command_source: CommandSource`, `focus_signal: Any` → `focus_signal: FocusSignal`, `progress_sink: Any` → `progress_sink: ProgressSink` trong các method: `_handle_commands`, `_process_wait_states`, `_service_control_state`, `_wait_until_runtime_deadline`, `run`. Sau cùng kiểm tra còn `Any` thừa: `grep -n ": Any" src/sky_music/orchestration/dispatch_loop.py` (chỉ nên còn các chỗ thực sự động, nếu có).

**Kiểm thử:** `uv run pyright && uv run pytest -q`. Pyright phải sạch (đây là phép thử chính của Phase E).
**Commit:** `refactor(dispatch): annotate IO seams with supervisor protocols`

---

## Phase G — Config (độc lập)

### G.1. Thêm ruff target-version
`pyproject.toml`, thêm khối (nếu chưa có `[tool.ruff]`):
```toml
[tool.ruff]
target-version = "py314"
```
### G.2. Quyết định cần con người (KHÔNG tự đổi nếu không có xác nhận)
`requires-python = ">=3.11,<3.15"` mâu thuẫn với `.python-version = 3.14+freethreaded`. Việc siết thành `">=3.14,<3.15"` là **quyết định của maintainer** (có thể phá môi trường chạy 3.11). → Ghi chú trong PR, **không tự siết** trừ khi được xác nhận.

**Kiểm thử:** `uv run ruff check .` (target mới có thể phát sinh cảnh báo upgrade idiom — sửa nếu ruff đề xuất `--fix` an toàn, hoặc báo lại).
**Commit:** `chore(ruff): pin target-version to py314`

---

## Phase F — Gỡ global mirror "legacy" (BLAST RADIUS LỚN — làm cuối, cẩn trọng)

**Mục tiêu:** Xóa `_sync_legacy_runtime_globals` và 14 global ở `main.py` (`:66-77`, `:113-131`), thay test bằng assert trên `RUNTIME_STATE`.

**Cảnh báo:** Đụng nhiều test. **Trước khi sửa**, liệt kê toàn bộ điểm phụ thuộc:
```
grep -rn "main\.\(TIMING_POLICY\|SLEEP_POLICY\|PLAYBACK_SESSION\|TELEMETRY_CSV_ENABLED\|DRY_RUN_MODE\|TEMPO_SCALE\|TIMING_PROFILE_NAME\|VERBOSE_HUD\|USE_DISPATCH_THREAD\|ENABLE_TIMER_GUARD\|ENABLE_WAITABLE_TIMER\|ENABLE_GC_PAUSE\|CURRENT_SCAN_CODE_MODE\)" tests/ src/
```
Điểm đã biết: `tests/test_cli.py:37-38`, `tests/test_acceptance_flow.py:30`, `tests/test_calibration.py` (nhiều dòng: 22-33, 47-50, 155-172, 234-261).

**Cách làm an toàn:**
1. Xác nhận **production không đọc** các global này (chỉ test đọc). Các global mà `main()` *thực sự* đọc (`PLAYBACK_SESSION`, `TEMPO_SCALE`, `CURRENT_SCAN_CODE_MODE`, `DRY_RUN_MODE`) → thay bằng `RUNTIME_STATE.<field>` tại điểm dùng trong `main()` (đọc `RUNTIME_STATE.session`, `.tempo_scale`, `.scan_code_mode`, `.dry_run`).
2. Đổi mỗi test assert `main.TIMING_POLICY` → `main.RUNTIME_STATE.timing_policy`, `main.SLEEP_POLICY` → `main.RUNTIME_STATE.sleep_policy`, `main.USE_DISPATCH_THREAD` → `main.RUNTIME_STATE.use_dispatch_thread`, v.v. (ánh xạ 1-1 theo `_sync_legacy_runtime_globals`).
3. Xóa `_sync_legacy_runtime_globals` và mọi lời gọi tới nó (`grep -n _sync_legacy_runtime_globals src/main.py`), xóa khối global khai báo `:66-77`.
4. Chạy `uv run pytest tests/test_cli.py tests/test_calibration.py tests/test_acceptance_flow.py -q` rồi trọn vẹn.

**Commit:** `refactor(main): drop legacy global mirror, assert on RUNTIME_STATE`

---

## Ngoài phạm vi (KHÔNG làm trong đợt refactor này)
- Telemetry partial-send / `chord_quantization_us` (rec #1 báo cáo) — là **tính năng mới**, không phải refactor.
- Sửa cột telemetry `deferred_by_us` luôn-0 — là **vấn đề hành vi**, cần quyết định riêng.
- Đổi `MAX_POLY`, `max_lead_us`, hằng `700/3000/100`, hay logic linear model — thuộc tinh chỉnh thuật toán, không phải dọn dẹp.

## Tiêu chí hoàn thành
- [ ] Mỗi Phase một commit, mỗi commit `uv run ruff check . && uv run pyright && uv run pytest` xanh.
- [ ] Số test pass ≥ baseline (Phase F có thể đổi *cách* assert nhưng không giảm số test).
- [ ] Không có thay đổi nào trong `scheduler.build_key_actions`, `_dispatch_down_batch`, `send_scan_code_batch*`, `wait_strategy.spin_until_us`, `rt_priority`, `RealtimeProcessScope`.
- [ ] `grep ": Any" src/sky_music/orchestration/dispatch_loop.py` không còn ở các IO seam đã liệt kê Phase E.

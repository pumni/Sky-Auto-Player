# Kế hoạch REFACTOR — Cho phép hold dài (âm sắc) mà giữ chuẩn nhịp

> **Bản 2/2.** Chỉ thực thi SAU khi `docs/hold-timbre-rhythm-findings.md` (sinh ra từ bản 1) đã có **VERDICT**.
> **Đối tượng thực thi:** AI coding agent.
> **Mục tiêu:** dựa trên kết luận đo được, cho phép nâng hold để âm sắc nghe hay hơn ở máy người nghe **mà không** xuất hiện lệch nhịp — bằng đúng cơ chế mà số liệu chỉ ra, không phải bằng phỏng đoán.
> **Trạng thái mã:** branch `main` sạch. Đọc lại mọi file trước khi sửa; số dòng có thể trôi.

---

## 0. Bắt buộc đọc trước

### 0.1. Ràng buộc bất biến (P0 — KHÔNG vi phạm)
- **SendInput ONLY.** Không hook/inject/đọc bộ nhớ game. Một hợp âm = một `KeyAction` = một `SendInput` (không đụng grouping trong `build_key_actions`, `_dispatch_down_batch`, `send_scan_code_batch*`).
- **Onset không bao giờ bị dời ở scheduler.** Mọi thay đổi không được làm `down_at_us` lệch khỏi thời điểm tác giả (giữ `test_onsets_are_not_shifted_or_clamped`, `test_degraded_same_key_behavior_timeline`).
- **Không tái sinh "margin tuỳ tiện".** `release_gap_us`/`repeat_release_gap_us` đã bị gỡ (timing-principles §6). Chỉ Nhánh C được đụng tới khe nhả, và **chỉ khi** bản 1 chứng minh nó bind.

### 0.2. Chọn nhánh theo VERDICT của bản 1
| VERDICT | Thực thi |
|---|---|
| **A** — H-CONTENTION xác nhận | **Nhánh A** (đổi ưu tiên drain sang onset-first) + **Nhánh chung** (mở hold qua config) |
| **B** — contention bị loại, nguyên nhân game/cảm nhận | **Nhánh B** (đo audio thật → chọn hold integer-frame; không sửa runtime) |
| **C** — same-key up-gap bind trên bài người dùng | **Nhánh C** (cap up-gap frame-aware) + **Nhánh chung** |
| A+C | làm cả A và C |

### 0.3. Lệnh kiểm thử (altitude)
| Phạm vi | Lệnh |
|---|---|
| Lint | `uv run ruff check .` |
| Type | `uv run pyright` |
| Test | `uv run pytest` |
| Trọn vẹn | `uv run ruff check . && uv run pyright && uv run pytest` |

Chạy trọn vẹn để xác nhận baseline XANH trước khi bắt đầu; ghi số test pass. Mỗi Phase = một commit, tự xanh trước khi sang Phase kế. Test đỏ ngoài dự kiến: **dừng, đọc, sửa gốc** — không nới assert.

### 0.4. "Định nghĩa hoàn thành" chung
Bất kỳ nhánh nào cũng phải kết thúc bằng **chạy lại harness bản 1** (`scripts/measure_hold_rhythm.py`) ở cùng cấu hình và **đính số liệu trước/sau** vào `docs/hold-timbre-rhythm-findings.md`. Tiêu chí thành công: ở mức hold cho âm sắc mong muốn (ví dụ 1.5f), `down_lat_p99` **không cao hơn** mức hold=1.0f quá ~½ frame.

### 0.5. Chân lý kiến trúc (P0 — bất biến vĩnh viễn)

> **Không làm tròn frame. Hướng đúng là tối ưu việc gửi phím sao cho game nhận đúng timeline — không phải thay đổi thời gian hold.**

Giải thích:
- **Hold là tham số âm sắc, không phải công cụ hiệu chỉnh nhịp.** Tăng hay giảm hold để "sửa" lệch nhịp là chẩn đoán sai nguyên nhân. Lệch nhịp phát sinh từ dispatch không đúng thời điểm, không phải từ hold ngắn hay dài.
- **Làm tròn frame (snap/align onset sang lưới frame local) là vô nghĩa** vì game lấy mẫu input trên render loop *riêng, không đồng bộ* với scheduler. Snapping tạo ra offset ngẫu nhiên thay vì loại bỏ nó. Cơ chế này đã bị retired (`frame_align` — xem timing-principles §6).
- **Hướng đúng duy nhất:** tối ưu pipeline `SendInput → game nhận` (adaptive lead, completion-anchor, spin-wait precision) để `down_dispatch_completed_us` tiệm cận `authored_at_us` nhất có thể. Khi onset đúng thời điểm, hold chỉ còn là quyết định âm sắc thuần túy.
- Mọi thay đổi tương lai về timing **phải chứng minh được** rằng nó giảm onset-lateness (đo bằng `down_lat_p99`) hoặc tăng độ nhất quán dispatch — không phải chỉ thay đổi một tham số và quan sát kết quả chủ quan.

---

## Nhánh chung — Mở `hold` qua config (cần cho A và C)

> Decouple hold↔min_hold **đã hoạt động sẵn** ở tầng `TimingPolicy.from_dict` (test `test_explicit_hold_declarations_remain_escape_hatches`). Việc còn lại chỉ là phơi nó ra cho người dùng một cách an toàn, có validate.

### CHUNG.1. Thêm profile thử nghiệm vào `config.py`
File `src/sky_music/config.py`, trong `DEFAULT_TIMING_PROFILES` (≈ dòng 80-103). **KHÔNG** sửa ba profile sẵn có. Thêm một profile mới, ví dụ:
```python
    "local_warm": {
        # Như local_precise (min_hold = đúng 1 frame, nhịp chắc) nhưng hold dài hơn cho âm sắc.
        # hold tách khỏi min_hold qua hold_frames; min_hold giữ 1.0 frame.
        "min_hold_frames": 1.0,
        "min_hold_unframed_us": 22000,
        "hold_frames": 1.5,            # <-- giá trị cần tinh chỉnh bằng đo audio
        "spin_threshold_us": 800,
        "focus_restore_grace_us": 50000,
    },
```
> `hold_frames` là escape hatch hợp lệ: `from_dict` đặt `hold_uses_frame_model=True` và materialise `hold_us = ceil(hold_frames × frame_us)` riêng, `min_hold_us` vẫn = `ceil(1.0 × frame_us)`.

### CHUNG.2. Cho phép chọn từ CLI/picker (nếu muốn người dùng A/B nhanh)
- `config.py`: thêm tên vào `CLI_PROFILE_NAMES` và `_PROFILE_KEY_TO_CLI` (≈ dòng 159-169) → `"local-warm"`. Cập nhật `normalize_profile_name` nếu cần alias.
- Kiểm tra `validate_builtin_timing_profile` / `validate_timing_profile` (`domain/validation.py`) chấp nhận profile mới: nó phải qua `validate_hold_ordering` (hold ≥ min_hold — đúng vì 1.5f > 1.0f) và ngưỡng an toàn min_hold. Chạy `test_timing_profile_validators`, `test_hold_ordering_invariant_rejects_hold_below_min_hold`.

### CHUNG.3. Kiểm thử
```
uv run pytest tests/test_scheduler_new.py tests/test_cli.py tests/test_calibration.py -q
uv run ruff check . && uv run pyright && uv run pytest
```
Thêm test khẳng định profile mới decouple đúng:
```python
def test_local_warm_decouples_hold_from_min_hold():
    p = FrameTimingPolicy.from_profile_name("local-warm", fps=60)
    assert p.min_hold_us == 16667          # ceil(1.0 * 16667)
    assert p.hold_us == 25001              # ceil(1.5 * 16667)
```
**Commit:** `feat(timing): add local-warm profile (hold decoupled from min_hold)`

> Lưu ý: bản thân CHUNG **không** sửa lệch nhịp — nó chỉ tạo phương tiện để nâng hold. Lệch nhịp được xử lý ở Nhánh A hoặc kết luận ở Nhánh B.

---

## Nhánh A — Onset-first drain (khi H-CONTENTION xác nhận)

**Nguyên nhân (theo VERDICT A):** dispatch đơn luồng đang xử lý `up` trước `down` trong mỗi vòng drain; hold dài đẩy các `up` lại gần onset phím khác → `up` chiếm SendInput trước → `down` (onset) bị trễ. Sửa = **ưu tiên onset khi cả hai cùng đến hạn**, vì độ chính xác của onset quan trọng hơn của release.

### A.0. Đọc & xác định điểm thật (đọc trước khi sửa)
- `src/sky_music/orchestration/dispatch_loop.py::_drain_due` (≈ dòng 851-880): hiện `pop_due_pending` (releases) chạy **trước** vòng down batches. Đây là nơi quyết định thứ tự runtime.
- `src/sky_music/orchestration/runtime_dispatch.py`: `pop_due_pending`, `pop_due_authored`, `next_deadline_us`, và **no-early-conflict guard** (timing-principles §3: một down batch không bao giờ được pop trước thời điểm tác giả khi phím của nó còn active hoặc còn pending release). Guard này là thứ giữ an toàn same-key, **độc lập** với thứ tự up/down → đổi thứ tự drain phím-khác không phá same-key.
- `scheduler.py:298` `sort(key=lambda a: (a.at_us, a.kind == "down"))`: đây là thứ tự **schedule** (tĩnh), KHÔNG phải thứ tự runtime drain. `test_normal_release_is_sorted_before_down_at_same_timestamp` khẳng định thứ tự schedule này → **GIỮ NGUYÊN scheduler**, chỉ đổi runtime drain.

### A.1. Đổi thứ tự drain: down đến hạn trước, up đến hạn sau
Trong `_drain_due`, đảo thứ tự: drain **due down batches trước**, rồi mới `pop_due_pending` cho releases trong cùng vòng. Yêu cầu:
- Giữ nguyên completion-anchor & lead (đừng đụng công thức `max(scheduled_release - lead, release_not_before)`).
- Giữ no-early-conflict guard. Nếu một due down bị guard chặn (phím còn chờ release của chính nó), **release đó phải được xử lý trước** down đó — tức guard vẫn thắng cho same-key. Triển khai: ưu tiên down phím-khác; với down mà guard chặn, để vòng sau (release của nó sẽ được drain và lần sau down đi được). KHÔNG bỏ guard.
- Không để release bị bỏ đói: một `up` đã quá hạn floor vẫn phải ra trong cùng vòng sau khi các down đến hạn đã đi (tránh kẹt phím / trễ release quá mức làm hold phình).

> Nếu việc đảo trong `_drain_due` rủi ro hơn dự kiến (đụng nhiều bất biến runtime), phương án thay thế nhỏ hơn: gộp due-downs và due-ups thành một danh sách, sort theo `(due_us, kind == "up")` (down trước up khi đồng hạn) rồi thực thi tuần tự. Chọn phương án ít blast-radius hơn sau khi đọc code.

### A.2. Kiểm thử (giữ xanh + chứng minh)
```
uv run pytest tests/test_runtime_dispatch.py tests/test_threaded_dispatch.py tests/test_adaptive_lead.py tests/test_engine_refactor.py -q
uv run ruff check . && uv run pyright && uv run pytest
```
- Bất biến phải xanh: completion-anchor (`test_dispatch_completion_lands_on_schedule_with_warm_estimator`), per-batch lead, không tăng `dropped_conflict`/stuck keys.
- Nếu có test khẳng định "release drained trước down" ở mức **runtime** (không phải schedule), đọc kỹ: nếu nó chỉ kiểm thứ tự cũ như một chi tiết triển khai (không phải hợp đồng same-key), cập nhật test theo hành vi mới + ghi lý do trong commit. Nếu nó bảo vệ an toàn same-key thật, **giữ** và điều chỉnh thiết kế A.1.

### A.3. CHỨNG MINH bằng harness bản 1 (bắt buộc)
Chạy lại:
```
uv run python scripts/measure_hold_rhythm.py "songs/<bai-da-thay-lech>.json"
```
So `down_lat_p95/p99` theo hold **trước vs sau** A.1. Thành công khi đường cong lateness-theo-hold **phẳng hẳn lại** (hold dài không còn làm onset trễ). Đính bảng vào findings.

**Commit:** `perf(dispatch): onset-first drain so long holds don't delay onsets`

---

## Nhánh B — Nguyên nhân ở phía game/cảm nhận (khi contention bị loại)

**Bối cảnh (VERDICT B):** harness cho thấy down-lateness phẳng theo hold → runtime **không** gây lệch. Lệch nhịp là hệ quả lượng tử hóa frame phía game: hold non-integer-frame làm độ dài nốt lúc N lúc N+1 frame (trọng tâm cảm nhận xê dịch). **Không có fix runtime** cho việc này; lượng tử cấm độ dài phân số nhất quán.

> Quan trọng: điều quyết định độ dài nốt mà *người nghe* cảm nhận là bội số nguyên của **frame phía AUDIENCE** (thường 60fps), không nhất thiết là FPS local. timing-principles §4 đã nêu cân nhắc audience.

### B.1. Đo audio thật (rank-1, thay cho phỏng đoán)
Dùng hạ tầng loopback có sẵn (`tests/audio_loopback.py`, `tests/measure_stutter_live.py`, `tests/analyze_onsets.py`) để thu onset thật trong game:
1. Phát cùng một đoạn với hold ∈ {1.0f, 1.5f, 2.0f} (qua profile `local-warm` đổi `hold_frames`), **FPS local = FPS audience mục tiêu** (vd cùng 60).
2. Thu onset/inter-onset-interval; tính độ lệch chuẩn IOI và "duration bimodality" (tỷ lệ nốt rơi vào 2-frame).
3. Lặp ở các bội số **nguyên** của audience-frame: hold ∈ {1.0f, 2.0f, 3.0f} để xác nhận chúng KHÔNG bimodal (kỳ vọng std IOI thấp đều).

### B.2. Kết luận khả dĩ và hành động
- Nếu một bội số nguyên (vd 2.0f tại audience-frame) cho âm sắc chấp nhận được **và** std IOI thấp → chốt `local-warm` ở giá trị đó; cập nhật `hold_frames` (CHUNG.1) thành số nguyên. Thêm test khẳng định `hold_us` là bội số nguyên của `frame_us` tại fps audience.
- Nếu **mọi** bội số nguyên >1 đều quá dài/đục (đúng như người dùng đã thấy với 2.0): kết luận trung thực là **âm sắc "hay hơn" của balanced gắn liền với jitter của nó; trong giới hạn SendInput-only, 1 frame là trần thực tế cho nhịp chắc.** Ghi điều này vào `docs/timing-principles.md` (mục mới "Hold integer-frame constraint") để chốt lại tri thức, tránh lặp thử nghiệm.
- Tuỳ chọn: thêm khuyến nghị calibration "đặt hold = 1 audience-frame khi chơi phòng online" vào đường calibrate (`orchestration/calibration.py`) — chỉ nếu B.1 ủng hộ.

### B.3. Kiểm thử
```
uv run pytest tests/test_calibration.py tests/test_scheduler_new.py -q
uv run ruff check . && uv run pyright && uv run pytest
```
**Commit (tuỳ kết quả):** `docs(timing): record integer-frame hold constraint from audio measurement` hoặc `feat(timing): finalize local-warm hold at <N> audience-frames`

> Nhánh B **không** được thêm cơ chế runtime "đoán" để bù frame-phase phía game (đã retired `frame_align`, timing-principles §6 — snapping vào lưới local vô nghĩa vì game lấy mẫu trên render loop riêng).

---

## Nhánh C — Cap up-gap frame-aware (CHỈ khi Phase 0 của bản 1 cho `binds=True`)

> ⚠️ Đây là tái sinh có kiểm soát của `repeat_release_gap_us` đã bị gỡ. **Chỉ làm** nếu `docs/hold-timbre-rhythm-findings.md` chứng minh same-key up-gap thật sự tụt < `frame_us` trên bài của người dùng. Nếu không, BỎ QUA nhánh này.

### C.1. Thêm ràng buộc up-gap vào `plan_same_key_hold`
File `src/sky_music/domain/scheduler.py`, hàm `plan_same_key_hold` (≈ dòng 83-118). Hiện tại nhánh "ok" trả `target_hold_us` không chặn dưới up-gap; nhánh "moderate" nén về đúng `effective_delta` (up_gap = 0).

Thêm tham số `frame_us` và cap để **luôn chừa ≥ 1 frame "up" sạch** trước cú gõ same-key kế:
```python
def plan_same_key_hold(
    *,
    target_hold_us: int,
    min_hold_us: int,
    effective_delta_us: int | None,
    frame_us: int = 0,            # MỚI: 0 = tắt cap (giữ hành vi cũ cho fps<=0/test cũ)
) -> PlannedKeyHold:
    if effective_delta_us is None:
        return PlannedKeyHold(hold_us=target_hold_us, risk="ok")

    max_hold_us = effective_delta_us
    feasibility_floor_us = min_hold_us
    if max_hold_us < feasibility_floor_us:
        return PlannedKeyHold(
            hold_us=min_hold_us, risk="severe",
            effective_delta_us=effective_delta_us,
            compressed=min_hold_us < target_hold_us,
        )

    # MỚI: chừa 1 frame nhả sạch trước cú gõ same-key kế, nhưng không bao giờ xuống dưới min_hold.
    gap_capped_target = target_hold_us
    if frame_us > 0:
        gap_capped_target = min(target_hold_us, max(min_hold_us, effective_delta_us - frame_us))

    if max_hold_us < gap_capped_target:
        return PlannedKeyHold(
            hold_us=max_hold_us, risk="moderate",
            effective_delta_us=effective_delta_us, compressed=True,
        )
    if gap_capped_target < target_hold_us:
        # bị cap để giữ up-gap; vẫn là một dạng nén có chủ đích
        return PlannedKeyHold(
            hold_us=gap_capped_target, risk="moderate",
            effective_delta_us=effective_delta_us, compressed=True,
        )
    return PlannedKeyHold(
        hold_us=target_hold_us, risk="ok",
        effective_delta_us=effective_delta_us,
    )
```
> Cap dùng `max(min_hold_us, ...)`: feasibility floor vẫn thắng — không bao giờ làm hold < min_hold. Nốt đơn (`effective_delta is None`) không bị cap → giữ âm sắc đầy.

### C.2. Truyền `frame_us` từ `build_key_actions`
Trong `build_key_actions` (Stage 2, ≈ dòng 217), `policy` là `FrameTimingPolicy` đã có `policy.frame_us`. Truyền vào:
```python
        planned_hold = plan_same_key_hold(
            target_hold_us=int(policy.hold_us),
            min_hold_us=int(policy.min_hold_us),
            effective_delta_us=effective_delta_us,
            frame_us=int(policy.frame_us),     # 0 khi fps<=0 -> cap tắt, hành vi cũ
        )
```

### C.3. Kiểm thử
- Test cũ truyền `plan_same_key_hold(... )` không có `frame_us` → mặc định 0 → hành vi cũ giữ nguyên (`test_plan_same_key_hold_*` xanh).
- Thêm test mới khẳng định cap kích hoạt khi fps>0 và interval gần hold:
```python
def test_plan_same_key_hold_reserves_one_frame_up_gap():
    planned = plan_same_key_hold(
        target_hold_us=25_000, min_hold_us=16_667,
        effective_delta_us=30_000, frame_us=16_667,
    )
    # cap = max(16667, 30000-16667=13333) -> 16667 < target 25000 => bị cap
    assert planned.hold_us == 16_667
    assert planned.compressed is True
```
- Kiểm tra `min_same_key_up_gap_us` trong metadata tăng lên ≥ `frame_us` cho bài bind; `down_at_us` KHÔNG đổi (`test_onsets_are_not_shifted_or_clamped`).
```
uv run pytest tests/test_scheduler_new.py -q
uv run ruff check . && uv run pyright && uv run pytest
```

### C.4. Chứng minh
Chạy lại `scripts/audit_same_key_gap.py` (bản 1): `min_up_gap` phải ≥ `frame_us` ở mọi mức hold sau khi sửa. Đính số liệu vào findings.

**Commit:** `feat(scheduler): reserve a one-frame up-gap for fast same-key repeats`

---

## Ngoài phạm vi (mọi nhánh)
- Đổi mô hình lead/estimator, grouping hợp âm, `wait_strategy.spin_until_us`, rt_priority, RealtimeProcessScope.
- Snapping onset vào lưới frame local (đã retired, vô nghĩa).
- "Margin" cố định kiểu cũ không gắn frame.

## Tiêu chí hoàn thành
- [ ] Đã chọn đúng nhánh theo VERDICT của bản 1.
- [ ] Mỗi Phase một commit, `uv run ruff check . && uv run pyright && uv run pytest` xanh; số test pass ≥ baseline.
- [ ] Onset không bị dời (`test_onsets_are_not_shifted_or_clamped` xanh).
- [ ] Đã chạy lại harness/audit bản 1 và đính số liệu **trước/sau** vào `docs/hold-timbre-rhythm-findings.md`, chứng minh nhịp không xấu đi ở mức hold cho âm sắc mong muốn.

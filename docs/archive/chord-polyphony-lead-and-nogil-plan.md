# Kế hoạch: Sửa sai pha hợp âm (polyphony-aware lead) + chuẩn hóa No-GIL

> Mục tiêu: hợp âm nhiều phím không còn "trễ/rời" khỏi giai điệu. Giữ nguyên timeline và
> hold ≥1 frame; chỉ điều chỉnh *thời điểm bắt đầu gửi* để mọi onset (đơn lẫn hợp âm) **hoàn tất ở
> cùng một pha frame**. Dự án đi theo chuẩn **No-GIL / free-threaded 3.14**.
> Tuân thủ AGENTS.md: scheduler thuần & test được, type hints, thêm test timing edge-case,
> 3 cổng `pytest`/`ruff`/`pyright` phải xanh.

---

## 0. Bằng chứng (đo từ `logs/playback_telemetry_20260625-190007-8707.csv`)

`visible_lateness` (lúc SendInput **hoàn tất** = cái game lấy mẫu) trôi đơn điệu theo số phím:

| poly | n | visible_lateness p50 | p95 | send_duration p50 | applied_lead p50 |
|---|---|---|---|---|---|
| 1 | 474 | **−24µs** | +49 | 236 | 297 (dư) |
| 2 | 208 | +17 | +174 | 373 | 426 |
| 3 | 300 | +100 | +327 | 490 | 431 |
| 4 | 103 | **+224µs** | +397 | 600 | 427 (thiếu 173) |

Phát hiện đã xác minh:
1. **Hợp âm gửi nguyên khối, không tách/rớt** (0/1085 `sent<intended`) → các phím trong hợp âm
   đồng thời với game. Vấn đề là hợp âm **as a whole** hoàn tất muộn, không phải lệch nội bộ.
2. **Gốc rễ = lead mù polyphony.** Lead là **một EMA chung (~400µs)** trong khi
   `send_duration_pure ≈ 110 + 121·N µs`. → nốt đơn lead dư (sớm), hợp âm lead thiếu (trễ).
3. **Nhiễu chéo qua EMA dùng chung:** nốt giai điệu ngay sau hợp âm bị lead dư → **−408µs (quá
   sớm)**. Biên dao động pha chord↔hàng xóm ≈ 600µs, **nhất quán dấu** → sát biên frame 16.67ms
   thì hợp âm liên tục lọt sang frame sau → nghe "rời".
4. `121µs/phím` chủ yếu là **Python per-key overhead bị no-GIL khuếch đại** (set-ops +
   atomic refcount), không phải OS SendInput (mảng INPUT đã cache, reuse 6.0×). → Không phải chi
   phí cứng; giảm được (Phase B).

**Kết luận:** lead phải phụ thuộc số phím của *chính batch đó*, và tách EMA theo cỡ.

---

## Phase A — Polyphony-aware adaptive lead (CHÍNH)

### A1. `SendLatencyEstimator` theo bucket polyphony
**File:** `src/sky_music/orchestration/engine.py` (class `SendLatencyEstimator`, ~dòng 43-100).

- Down: thay 1 EMA bằng **EMA riêng theo số phím** `N` (bucket). Clamp `N` vào `[1, MAX_POLY]`
  (đề xuất `MAX_POLY=6`). Mỗi bucket có seed/đếm/EMA riêng như cơ chế hiện tại
  (`_SEED_SAMPLES`, `alpha`).
- API mới (giữ tương thích chữ ký cũ qua tham số mặc định):
  - `update(kind, duration_us, n_keys: int = 1)`
  - `get_lead_us(kind, n_keys: int = 1) -> int`
- **Fallback cho bucket chưa seed** (vd poly5 hiếm): dùng bucket đã seed **gần nhất ≤ N**; nếu
  không có, dùng EMA down tổng (giữ một EMA tổng phụ); nếu vẫn cold → `0` (giữ hành vi cold hiện
  tại: chưa đủ mẫu thì lead 0).
- Giữ `max_lead_us=2000` (đủ cho ~600µs). Up (release): **giữ nguyên 1 EMA scalar** ở phase này.
- Tự chọn: thay vì bucket có thể dùng **hồi quy tuyến tính online** `a + b·N` (data rất tuyến tính:
  a≈110, b≈121). Nếu chọn, phải clamp `[0, max_lead]` và chặn mẫu số. **Khuyến nghị bucket** vì
  đơn giản, dễ test, không lo số học.

### A2. Đưa N vào đường tính lead — per-batch, không phải scalar
**File:** `src/sky_music/orchestration/runtime_dispatch.py` (`RuntimeDispatchCoordinator`).

Vấn đề: `next_authored_us` / `pop_due_authored` / `next_deadline_us` đang nhận **lead scalar**, nhưng
`pop_due_authored` pop **nhiều batch** mỗi lần → mỗi batch cần lead theo cỡ riêng.

- Thêm tham số **keyword optional** `lead_for_batch: Callable[[RuntimeActionBatch], int] | None`
  vào 3 hàm trên, **giữ nguyên tham số scalar cũ** để test hiện tại không vỡ:
  - Nếu `lead_for_batch` được cung cấp → dùng `lead = lead_for_batch(batch)` cho **từng batch**
    trong điều kiện pop (`scheduled_us - lead <= now_us`, tương đương `scheduled_us <= now + lead`)
    và trong `next_authored_us` cho batch ở `cursor`.
  - Nếu `None` → giữ y nguyên hành vi scalar cũ.
- **Giữ nguyên guard `_early_pop_blocked`**: khi bị chặn vẫn trả `scheduled_us` (không lead). Logic
  guard không đổi, chỉ thay nguồn `lead`.
- Coordinator vẫn **thuần**: nó chỉ gọi callable được tiêm, không đọc clock.

### A3. DispatchLoop cung cấp `lead_for_batch`
**File:** `src/sky_music/orchestration/dispatch_loop.py`.

- Thêm method:
  ```python
  def _down_lead_for_batch(self, batch: RuntimeActionBatch) -> int:
      if batch.kind != "down":
          return 0  # up authored batches: release timing do pending lead_up xử lý
      if self.dispatch_lead_us > 0:
          return self.dispatch_lead_us + self.onset_bias_us      # --dispatch-lead-us: scalar cố định
      base = 0
      if self.enable_adaptive_lead and self.estimator is not None:
          base = self.estimator.get_lead_us("down", len(batch.intents))
      return base + self.onset_bias_us
  ```
- `run()` / `_drain_due()`:
  - Truyền `lead_for_batch=self._down_lead_for_batch` vào `next_deadline_us`, `next_authored_us`,
    `pop_due_authored`. `lead_up` (pending releases) giữ scalar như cũ.
  - `_dispatch_down_batch`: tính `applied_lead = self._down_lead_for_batch(batch)` để ghi telemetry
    `applied_lead_us` đúng theo cỡ (thay vì lead_down scalar).
- `estimator.update`: tại `_dispatch_down_batch`, sau dispatch gọi
  `self.estimator.update("down", result.send_duration_us, n_keys=len(playable))`
  (giữ guard `enable_adaptive_lead and estimator is not None`). Dùng `len(playable)` (số phím thực
  gửi) cho khớp với `send_duration` đo được.
- `get_current_leads()` vẫn tồn tại cho đường `lead_up`/pending; phần down scalar trở thành phụ
  (chỉ dùng nơi không có batch cụ thể). Không xóa để tránh churn; nhưng **đường quyết định down giờ
  đi qua `_down_lead_for_batch`**.

### A4. Bảo toàn hành vi & giới hạn
- `--dispatch-lead-us > 0`: bỏ qua polyphony (scalar cố định) — giữ nguyên ngữ nghĩa CLI.
- `enable_adaptive_lead=False` và lead=0: lead = `onset_bias_us` (thường 0) cho mọi cỡ — như hiện tại.
- **Không đụng** timeline, hold, min_hold, completion-anchor.
- Releases (up) **ngoài phạm vi** phase này (ghi chú để phase sau cân nhắc bucket cho up).

### A5. Tests (bắt buộc, theo AGENTS.md)
Thêm/chỉnh trong `tests/test_adaptive_lead.py` và `tests/test_runtime_dispatch.py`:
1. **Estimator:** seed riêng từng bucket; `get_lead_us("down",4) > get_lead_us("down",1)` sau khi
   nạp mẫu lớn cho bucket 4; fallback bucket chưa seed → bucket ≤N gần nhất; cold → 0; clamp
   `max_lead_us`.
2. **Coordinator (thuần, fake clock):** với `lead_for_batch` tiêm (vd poly→`100*N`), một batch poly4
   pop **sớm hơn** poly1 đúng theo lead; `next_authored_us` phản ánh lead của batch ở cursor;
   `_early_pop_blocked` vẫn chặn (lead không vượt guard khi phím còn active/pending).
3. **DispatchLoop tích hợp (fake clock/estimator):** chuỗi [nốt đơn, hợp âm 4] → hợp âm được
   `actual_us`/deadline sớm hơn tương ứng; `applied_lead_us` telemetry = lead theo cỡ.
4. **Regression:** mọi test cũ xanh; các call-site `update`/`get_lead_us` cập nhật tham số `n_keys`.

### A6. Tiêu chí nghiệm thu (định lượng)
- 3 cổng xanh: `uv run pytest`, `uv run ruff check .`, `uv run pyright`.
- Phát lại đúng bài với `--debug-csv`, chạy script ở §3 dưới. **Mục tiêu:**
  - `visible_lateness p50` của poly1..poly4 **phẳng**, chênh `|poly4 − poly1| < 50µs` (hiện 248µs).
  - Nốt-ngay-sau-hợp-âm: `lateness(start)` không còn lệch (hiện −408µs sau poly4 vs −243µs sau
    poly1) — chênh `< ~80µs`.
  - `over_2ms` vẫn 0; không phát sinh drop/conflict (`sender_clean=true`).

---

## Phase B — Giảm độ dốc per-phím (PHỤ, tối ưu no-GIL, làm sau Phase A)

**File:** `src/sky_music/infrastructure/backend.py` (`_TrackedKeyState._decide_down/_decide_up`,
`key_down/key_up`).

- Mục tiêu: hạ `~121µs/phím` bằng cách bớt set-ops & đối tượng tạm trên hot path (gộp thao tác cả
  batch; fast-path "toàn phím mới" đã có ở `_decide_down`, mở rộng cho `_decide_up`; giảm số lần
  `possibly/active.update` + `difference_update`).
- **Cổng:** chỉ merge nếu `scripts/measure_dispatch_tail.py` (hoặc script §3) cho thấy slope
  `send_duration_pure` giảm rõ mà **không** đổi hành vi (state tracking, panic-release, idempotency).
- Giữ test `test_runtime_dispatch` (cache/retry SendInput) xanh; thêm test cho fast-path mới.
- **Không bundle với Phase A** — PR/commit riêng để dễ review và đo tách bạch.

---

## Phase C — Chuẩn hóa No-GIL (config, độc lập)

- **Pin interpreter free-threaded** rõ ràng để uv không chọn nhầm: `.python-version` → `3.14t`
  (hoặc selector `cpython-3.14+freethreaded`). Xác nhận `uv run python -c "import sys;
  print(sys._is_gil_enabled())"` in `False`.
- **Kiểm tra wheel free-threaded** của dependency C-ext **`rapidfuzz`** (chỉ dùng ở picker, không ở
  đường dispatch). Nếu thiếu wheel 3.14t → ghi rủi ro + phương án (lazy import đã cô lập ở picker).
- Giữ các guard `_gil_enabled()` sẵn có (realtime.py); switch-interval tuning tự bỏ khi no-GIL.
- Ghi chú vào AGENTS.md/README: dự án chạy chuẩn **free-threaded 3.14**; chi phí per-note cao hơn
  được bù bằng polyphony-aware lead (Phase A).
- **Commit riêng** với Phase A/B.

---

## 3. Script kiểm chứng (chạy trước/sau Phase A)

Lưu thành `scripts/audit_polyphony_lead.py` (hoặc chạy ad-hoc). Đặt `sys.stdout.reconfigure("utf-8")`.
Đọc CSV, nhóm `down` theo số scan-code, in `visible_lateness` p50/p95 theo poly, và lateness của
nốt-ngay-sau theo cỡ chord đứng trước. (Logic đã dùng trong phân tích — tái sử dụng.)

Bảng "trước" để đối chiếu: poly1 −24 / poly2 +17 / poly3 +100 / poly4 +224 (visible p50, µs);
sau-chord lateness: −243 (sau poly1) → −408 (sau poly4).

---

## 4. Thứ tự & guardrail

| Bước | Nội dung | Rủi ro | Cổng |
|---|---|---|---|
| A1 | Estimator theo bucket | Thấp | `test_adaptive_lead` |
| A2-A3 | Per-batch lead qua coordinator + loop | TB | `test_runtime_dispatch`, `test_threaded_dispatch` |
| A5 | Tests timing edge-case | — | pytest/ruff/pyright |
| A6 | Đo lại bằng §3 | — | bảng phẳng |
| B | Giảm slope backend | TB | bench + tests |
| C | Pin no-GIL | Thấp | `_is_gil_enabled()==False`, suite xanh |

**Bắt buộc:** giữ scheduler thuần (coordinator chỉ nhận callable, không đọc clock); type hints đầy
đủ; không broad rewrite; mỗi phase một commit; chạy đủ 3 cổng trước khi báo xong. Nếu test đỏ →
truy nguyên gốc, **không** nới lỏng assert.

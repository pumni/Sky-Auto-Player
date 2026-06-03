# Timing Architecture Audit — chỉ số nào THẬT, chỉ số nào bỏ

> Mục đích: phân loại từng tham số timing theo **tác dụng thật đã đo/đã chứng minh**, để dọn sạch
> các cần gạt "nêu ra nhưng vô dụng" (lãng phí + khó maintain) TRƯỚC khi tinh chỉnh giá trị.
> Đây là bản thiết kế cho đợt refactor — chưa implement. Nguồn số đo: [`timing-experiments.md`](timing-experiments.md)
> (Part 1 T1–T4) và [`result.md`](result.md) (kết quả thực đo của user, 2026-06).

---

## 0. Nguyên tắc phân loại

Một chỉ số chỉ được giữ nếu nó **làm thay đổi hành vi quan sát được** trên máy thật, ở bài hát thật.
"Có trong code và được trừ/cộng ở đâu đó" KHÔNG đủ — phải hỏi: *nếu xoá nó, người nghe có nhận ra
khác biệt không?* Nếu không → nó là nhiễu kiến trúc.

---

## 1. Phát hiện trọng tâm: `input_lead` là no-op theo kiến trúc

### 1.1 Cơ chế (đã chứng minh bằng code + chạy thử)

- Engine **zero-base** đồng hồ về thời điểm bấm play: `start_perf = clock.now_us()`
  (`orchestration/engine.py:286`). Action chạy khi `elapsed_us >= action.at_us`, với `elapsed` đo
  từ mốc 0 = lúc play.
- Scheduler dịch mọi nốt sớm lên một lượng cố định rồi **clamp nốt đầu**:
  `shifted_time_us = max(0, target_snapped_time - policy.input_lead_us)` (`domain/scheduler.py:139`).
- Player **tự sinh toàn bộ timeline**, KHÔNG có mốc tham chiếu ngoài (không backing track, không
  metronome trong vòng phát). Một phép **dịch đều toàn timeline là không quan sát được** — vì người
  dùng bấm play lúc nào thì mốc 0 ở đó.

### 1.2 Bằng chứng chạy thử (4 nốt, cách 500ms, local_precise @60fps)

```
lead=     0: gaps=[500000, 500000, 500000]   ← chuẩn
lead=  4000: gaps=[496000, 500000, 500000]   ← chỉ gap ĐẦU bị nén 4ms
lead= 20000: gaps=[480000, 500000, 500000]   ← chỉ gap ĐẦU bị nén 20ms
```

Vì nốt đầu (source=0) bị `max(0, …)` ghim về 0 trong khi các nốt sau bị kéo sớm đều → **chỉ khoảng
cách ĐẦU TIÊN bị nén đúng bằng lead**; mọi khoảng sau không đổi. Nếu nốt đầu có offset > lead thì
ngay cả gap đầu cũng không đổi → lead **hoàn toàn vô hình**.

### 1.3 Đối chiếu thực đo (O1, result.md)

User đo `--input-lead-ms` ∈ {0, 8, 20} bằng metronome reference: **mọi giá trị cho offset như nhau**
(~20ms sớm với latency-comp −20ms). Khớp kết luận: offset ~20ms là của DAW/tick game (T3), **không
phải do lead**. Remote (O1) cũng kết luận không nên thêm lead — "xử lý tốt local thì remote cũng tốt".

### 1.4 Phán quyết

`input_lead` **không phải "đến sớm" theo nghĩa nhạc lý — nó là no-op**, kèm một **lỗi nhẹ**: với
bài bắt đầu ở t=0, clamp làm **nén khoảng mở đầu đúng bằng lead** (audience 14500 → nốt đầu lệch
14.5ms). → **BỎ.** Đây là cần gạt nêu ra nhưng vô dụng điển hình.

---

## 2. Bảng audit toàn bộ chỉ số timing

| Chỉ số | Tác dụng thật? | Phán quyết | Bằng chứng / lý do |
| --- | --- | --- | --- |
| `min_hold` (sàn visibility 1 frame) | ✅ Cốt lõi | **GIỮ** + tinh chỉnh ratio | T1: sàn thật ~1.0f; code 1.25f |
| `repeat_release_gap` (sàn same-key) | ✅ Cốt lõi | **GIỮ** (đã chuẩn) | T2: 25001@60 ≈ đo 24ms; 18000@144 ≈ đo 16ms |
| `hold` (trần note-down) | ⚠️ Local `hold==min_hold` mọi FPS | **GỘP** vào min_hold (local) | Sky kêu khi key-DOWN; độ dài hold không đổi onset đơn |
| `input_lead` | ❌ No-op + méo nốt đầu | **BỎ** | §1 ở trên + O1 |
| `chord_merge` | ❌ Gần như không fire | **BỎ** (chord trùng giờ vẫn gom ở step 6) | O2; bài thật không có cụm 5–20ms |
| `frame_align` (`down_only`) | ❌ Off mọi profile, vô nghĩa | **BỎ** | game tự sample theo frame riêng của nó |
| `hold_unframed` / `min_hold_unframed` | ⚠️ Chỉ dùng khi fps=None | **GỘP** (chỉ còn raw escape hatch) | raw mode = cổng thí nghiệm |
| `release_gap` (defer release đụng down) | ✅ Có, nhỏ | **GIỮ** | `scheduler.py:255` |
| `focus_restore_grace`, `spin_threshold` | ✅ Infra | **GIỮ** | khác concern, không chạm onset |
| `same_key_conflict_policy` | ✅ strict/degraded | **GIỮ** | gate impossible repeat |
| ratio `input_lead_min_frame_ratio` (<60 raise lead) | ❌ Chết theo lead | **BỎ** | phụ thuộc input_lead |
| ratio `chord_merge_max_frame_ratio` | ❌ Chết theo chord_merge | **BỎ** | phụ thuộc chord_merge |

### Mô hình local sau dọn dẹp = đúng 3 cần gạt thật

1. **`min_hold`** — sàn 1-frame (visibility). Tinh chỉnh 1.25 → ~1.0–1.1 theo T1.
2. **`repeat_release_gap`** — `max(1.5×frame, ~17ms)`. Đã khớp T2, giữ.
3. **`release_gap`** — defer release nhỏ khi đụng down kế. Giữ.

Cộng hạ tầng: `focus_restore_grace`, `spin_threshold`, `same_key_conflict_policy`. Mọi thứ khác là
nhiễu.

---

## 3. Tinh chỉnh local sát số đo (T1/T2)

- `min_visible_hold_frames` / `min_hold_min_frame_ratio`: **1.25 → 1.1**.
  T1: ở 60fps, 16ms (0.96f) vẫn 15/15; rớt từ 15ms (0.90f). 1.1f còn ~15% biên trên mép rớt thật →
  sắc hơn ~4ms@60 mà gần như không rủi ro.
- `repeat_release_gap_floor_us`: **18000 → 16500** (tuỳ chọn). T2 @144 đo 16ms reliable; giữ chút biên.
  Không bắt buộc — 18000 chỉ dư 2ms.

> Đây đúng là item §16 ("frame-relative local hold tuning, tách riêng, cần đo in-game") trong
> [`timing-profile-frame-model.md`](timing-profile-frame-model.md). Giờ đã có dữ liệu T1 để quyết.

---

## 4. "Local đôi khi mất nốt" — không phải bug timing

Vì `hold == min_hold` ở mọi FPS cho local_precise, profile này **không có dải "risky"** — chỉ nhị
phân ok/impossible. Nốt rớt ở local **chỉ đến từ same-key repeat nhanh hơn min cycle**
(45.8ms @60 = 21.8 nốt/giây cùng phím). Khi vượt ngưỡng, hai lần bấm chồng lên nhau → game thấy một
hold liên tục → mất onset thứ hai. **Đây là giới hạn vật lý của game; tool flag `impossible` là
đúng.** Sửa bằng nhạc (giãn tempo / đổi voicing), không phải bằng param.

local_precise resolved (xác nhận bằng `FrameTimingPolicy.local_precise(fps=…)`):

| FPS | hold | min_hold | repeat_gap | min cycle | max repeat |
| --: | ---: | ---: | ---: | ---: | ---: |
| 30 | 41667 | 41667 | 50000 | 91.7ms | 10.9/s |
| 60 | 20834 | 20834 | 25001 | 45.8ms | 21.8/s |
| 144 | 8680 | 8680 | 18000 | 26.7ms | 37.5/s |

---

## 5. Lộ trình refactor (3 phase, ngắt được)

- **Phase 1 — Bỏ `input_lead`** (tác động lớn nhất, bằng chứng chắc nhất):
  xoá field policy + CLI `--input-lead-ms` + clamp `max(0, … - lead)` (đổi thành không dịch) +
  nhánh `<60` raise lead + telemetry/calibration recommend lead + ratio `input_lead_min_frame_ratio`.
  Đụng ~8 file (config, scheduler_types, scheduler, session_context, validation, telemetry,
  calibration, main + tests).
- **Phase 2 — Bỏ `chord_merge` + `frame_align`**:
  xoá block merge (chord cùng timestamp vẫn gom ở step 6 grouped-by-at_us), xoá `frame_align` +
  `align_frame_down_us` + ratio `chord_merge_max_frame_ratio`. Đụng ~6 file.
- **Phase 3 — Gộp `hold`/`min_hold` + tinh chỉnh ratio**:
  một sàn visibility duy nhất cho local, bỏ `*_unframed`, hạ ratio 1.25 → 1.1.

Mỗi phase: chạy full test + verify `resolve_effective_policy` không đổi ngoài phần cố ý.

### Lưu ý an toàn

- `chord_merge=0` vẫn xử lý đúng **chord trùng timestamp** (gom ở step 6). Chỉ mất khả năng gom nốt
  cách nhau 5–20ms — vốn không xuất hiện trong bài thật.
- `audience_safe` đang dựa vào `input_lead` làm điểm phân biệt chính. Bỏ lead → audience cần lý do
  tồn tại mới (chỉ còn sàn hold/repeat rộng hơn). Cân nhắc khi tới Phase 1: hoặc giữ audience bằng
  sàn, hoặc đánh giá lại sự cần thiết của profile này (xem O3/O4 — đang gác để làm local trước).
- Bỏ field timing là **breaking change** với `config.json` của user có override các field đó. Cần
  xử lý graceful (bỏ qua key lạ, đã có sẵn cơ chế ở `merged_timing_profiles`).

---

## 6. Handoff & Acceptance (cho AI thực thi + người nghiệm thu)

> Refactor này do một AI khác thực thi; một bên khác nghiệm thu. Phần này là **hợp đồng**: baseline
> đông cứng + cổng nghiệm thu cơ học cho từng phase. Bên thực thi PHẢI làm pass mọi cổng; đừng đổi
> giá trị nào ngoài phần "thay đổi có chủ đích" của phase đang làm.

### 6.0 Baseline đông cứng (chụp trước refactor, 2026-06)

> **TIẾN ĐỘ — TẤT CẢ 3 PHASE ĐÃ NGHIỆM THU PASS (2026-06):**
> Phase 1 (bỏ input_lead) ✅ · Phase 2 (bỏ chord_merge + frame_align) ✅ · Phase 3 (gộp hold/min_hold
> + local 1.1f + gộp ScheduledNoteDraft) ✅. Test: 176 → **171 passed**. Mô hình local còn đúng 3 cần
> gạt thật (min_hold/visibility, repeat_gap, release_gap) + hạ tầng. `ScheduledNoteDraft` còn một
> trường thời gian (`at_us`). local_precise visibility @60 = 18334 (1.1f), sắc hơn cũ 2.5ms, vẫn trên
> sàn đo 16ms.


- **Test suite: `176 passed`** (`uv run python -m pytest -q`). Sau mỗi phase phải xanh lại (số test
  có thể GIẢM khi xoá test của field bị bỏ — đó là hợp lệ; KHÔNG được có test fail).
- **Bảng resolved policy** (lệnh tái lập ở 6.4). Mọi cột KHÔNG thuộc "thay đổi có chủ đích" của phase
  phải **y hệt baseline**:

```
profile        fps   hold   min_hold  rgap   rel_gap  lead   chord  align
local_precise  None  22000  17000     18000  3500     4000   2500   none
local_precise  30    41667  41667     50000  5000     16667  2500   none
local_precise  60    20834  20834     25001  3500     4000   2500   none
local_precise  144   8680   8680      18000  3500     4000   2500   none
balanced       60    20834  20834     25001  4000     6000   3000   none
balanced       144   14000  14000     18000  4000     6000   3000   none
dense_safe     60    20834  20834     25001  5000     7000   4000   none
dense_safe     144   11000  11000     18000  5000     7000   4000   none
audience_safe  None  20000  18000     24000  9000     10000  5000   none
audience_safe  60    20834  20834     25001  9000     10000  5000   none
audience_safe  144   20000  18000     24000  9000     10000  5000   none
```

- **Onset baseline** (4 nốt cách 500ms, local_precise @60): hiện `gaps=[496000,500000,500000]`
  (gap đầu bị nén bởi lead 4000 — chính là artifact cần xoá ở Phase 1).

### 6.1 Cổng nghiệm thu — Phase 1 (bỏ `input_lead`)

1. **Grep sạch:** không còn `input_lead`, `input_lead_us`, `--input-lead-ms`, `input_lead_min_frame_ratio`,
   `base_input_lead_us` trong `src/` (chỉ được còn ở docs lịch sử nếu có chú thích).
2. **Onset không còn dịch/nén:** với bài nốt ở [0,500,1000,1500]ms, local_precise @60 →
   `down = [0, 500000, 1000000, 1500000]`, **mọi gap = 500000** (gap đầu KHÔNG còn bị nén). Kiểm cả
   bài bắt đầu ở t>0 → không clamp, không dịch.
3. **Equivalence:** bảng 6.0 với cột `lead` bị xoá — **mọi cột còn lại y hệt** ở cả 4 profile ×
   {None,30,60,144}.
4. `uv run python -m pytest -q` xanh.
5. **Không còn dead code:** telemetry/calibration không còn "recommend input_lead"; nhánh `<60` không
   còn raise lead.

### 6.2 Cổng nghiệm thu — Phase 2 (bỏ `chord_merge` + `frame_align`)

1. **Grep sạch:** không còn `chord_merge`, `chord_merge_window`, `--chord-merge-window-ms`,
   `frame_align`, `align_frame_down_us`, `chord_merge_max_frame_ratio` trong `src/`.
2. **Chord trùng timestamp vẫn gom:** 3 nốt khác phím cùng `time_ms` → **một** KeyAction `down` với 3
   scan_code (gom ở scheduler step 6 grouped-by-`at_us`).
3. **Thay đổi có chủ đích (OK):** nốt cách nhau 5–20ms giờ gửi ở mốc riêng (không gom) — đây là kết
   quả mong muốn, không phải regression.
4. **Equivalence:** cột `hold/min_hold/rgap/rel_gap` của bảng 6.0 không đổi.
5. `pytest -q` xanh.

### 6.3 Cổng nghiệm thu — Phase 3 (gộp `hold`/`min_hold` + hạ ratio)

> **QUYẾT ĐỊNH ĐÃ CHỐT (user, 2026-06):** hạ ratio visibility `1.25 → 1.1` **CHỈ cho `local_precise`**.
> `balanced` / `dense_safe` / `audience_safe` **giữ nguyên 1.25** (cả 3 set `hold_frames`/`min_hold_frames`
> tường minh trong profile dict, nên chỉ cần đổi của local_precise — KHÔNG đổi `FrameTimingDefaults`
> global). Phạm vi này giữ thang bậc profile rõ: local sắc nhất, các profile khác bảo thủ hơn.

**Thay đổi có chủ đích — CHỈ cột visibility của `local_precise` đổi:**

| FPS | local_precise visibility (hold==min_hold) MỚI | (baseline cũ 1.25f) |
| --: | --: | --: |
| 30 | **36667** | 41667 |
| 60 | **18334** | 20834 |
| 144 | **7639** | 8680 |

1. **Cấu trúc:** `local_precise` có MỘT field sàn visibility (không còn cặp `hold`/`min_hold` trùng giá
   trị tình cờ). Gộp `ScheduledNoteDraft` 4 trường thời gian (`source`/`snapped`/`shifted`/`down`, giờ
   bằng nhau hệt sau Phase 1+2) về **một** trường + bỏ biến trung gian `target_snapped_time` + sửa
   tên `merged_drafts`/`snapped`/`shifted` (gây hiểu nhầm) + đánh số lại comment (đang nhảy 2→3→5→6).
2. **Giá trị:** local_precise @{30,60,144} khớp bảng trên. **3 profile kia + repeat_gap + release_gap
   của MỌI profile = y hệt baseline §6.0.**
3. **fps=None (raw escape hatch):** đây là chỗ DUY NHẤT hiện `hold≠min_hold` (22000 vs 17000). Gộp ép
   một giá trị raw — bên thực thi chọn một số, đảm bảo `hold==min_hold`, và GHI rõ lý do; nghiệm thu
   xác nhận bất biến `hold==min_hold` ở raw.
4. `validate_hold_ordering` + invariant `min_hold_frames >= 1.0` vẫn pass (1.1 ≥ 1.0 ✓).
5. `pytest -q` xanh.

### 6.4 Lệnh tái lập bảng resolved (dùng để equivalence-check mọi phase)

```
uv run python -c "
from sky_music.domain.scheduler_types import FrameTimingPolicy
for name in ['local_precise','balanced','dense_safe','audience_safe']:
    for fps in (None,30,60,144):
        p=getattr(FrameTimingPolicy,name)(fps=fps)
        print(name,fps,p.hold_us,p.min_hold_us,p.repeat_release_gap_us,p.release_gap_us,p.frame_align)
"
```

### 6.5 Quy tắc cho bên thực thi

- Làm **từng phase một**, commit riêng, chạy `pytest` sau mỗi phase.
- KHÔNG gộp thay đổi giá trị (Phase 3) vào lúc xoá field (Phase 1/2) — giữ mỗi commit một ý.
- Field bị bỏ trong `config.json` của user: xử lý **graceful** (bỏ qua key lạ — `merged_timing_profiles`
  đã có cơ chế), KHÔNG raise.
- Nếu phát hiện cổng nào không thể pass mà không đổi behavior ngoài ý định → **dừng, báo lại**, đừng
  tự nới tiêu chí.

---

## 7. Liên quan

- Số đo gốc: [`timing-experiments.md`](timing-experiments.md), [`result.md`](result.md)
- Mô hình giá trị: [`timing-profile-frame-model.md`](timing-profile-frame-model.md)
- Nguyên tắc & Appendix A: [`timing-principles.md`](timing-principles.md)

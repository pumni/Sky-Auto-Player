> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: kế hoạch loại bỏ hold floor cũ đã thực thi.

# Floor Removal + 3-Profile Pure-Frame Model — Handoff Contract

Date: 2026-06-04
Planner/Reviewer: planning AI · Executor: another AI

> **Quyết định của user (đã chốt qua hỏi-đáp):**
> 1. **Xoá hẳn `min_hold_floor_us`** (và toàn bộ khái niệm "floor" trong materialisation). Lý do user
>    nêu + đã xác nhận bằng số: floor âm thầm biến profile "1.2 frame" thành **~2 frame ở FPS cao**
>    (balanced floor 14000 = 2.02 frame @144; audience 18000 = 2.59 frame @144) — số chạy KHÁC số khai
>    báo, không nhất quán.
> 2. **Mô hình mới = THUẦN frame-relative:** `min_hold_us = ceil(frames × frame_us)` khi fps>0;
>    `min_hold_unframed_us` khi fps=None. Không còn `max(…, floor)`.
> 3. **Còn 3 profile** (xoá `dense_safe`): **local_precise = 1.0 · audience_safe = 1.05 · balanced = 1.2**
>    (đều `min_hold_frames`). `hold` tiếp tục derive = `min_hold` (giữ nguyên cơ chế unification đã làm).
> 4. **local_precise trả về 1.0** — đồng thời giải quyết REJECT trước đó (executor đã sai khi đổi 1→1.05).

> **Rủi ro đã nêu & user chấp nhận có chủ ý:** bỏ floor ⇒ ở **FPS-local cao**, hold tuyệt đối ngắn lại
> (audience @144 = **7293µs ≈ 7.3ms**). Người nghe **remote ~60fps** (frame 16.7ms) có thể lọt mất —
> đây vốn là việc floor gánh (docs EXP-4 khuyên "hạ có kiểm chứng, đừng xoá hẳn"). User đi xa hơn docs
> dựa trên A/B test nghe thật. Ghi rõ trong docs khi cập nhật, KHÔNG tự thêm lại floor.

---

## 0. Bất biến phải giữ
1. **Escape hatch còn sống:** CLI `--hold-ms`/`--min-hold-ms`, explicit `hold_us`/`min_hold_us` trong
   config, và đường `fps=None` (`*_unframed_us`). `plan_same_key_hold` + band nén qua override **không
   đụng một dòng**.
2. **Golden bất biến:** `tests/test_golden_regression.py` build bằng `TimingPolicy.from_dict({})`
   (= balanced @fps=None) — KHÔNG dùng floor/dense/local. Phải xanh, KHÔNG regenerate.
3. **Same-key feasibility** vẫn do `min_hold_us` quyết. `min_hold_frames >= 1.0` (≥1 frame visibility).
4. **Graceful** với config.json cũ còn `dense_safe`/`*_floor_us`: bỏ qua key lạ, canonical fallback
   `balanced` (đã có sẵn) — KHÔNG raise.

---

## 1. Baseline MỚI (mục tiêu — dùng để equivalence-check)

Lệnh tái lập (chỉ còn 3 profile):
```
uv run python -c "
from sky_music.domain.scheduler_types import FrameTimingPolicy
for name in ['local_precise','balanced','audience_safe']:
    for fps in (None,30,60,144):
        p=getattr(FrameTimingPolicy,name)(fps=fps)
        print(name,fps,p.hold_us,p.min_hold_us)
"
```
Giá trị đích (hold==min mọi dòng):
```
local_precise  None 22000 22000   30 33334 33334   60 16667 16667   144 6945 6945
audience_safe  None 18000 18000   30 35001 35001   60 17501 17501   144 7293 7293
balanced       None 17000 17000   30 40001 40001   60 20001 20001   144 8334 8334
```

### Thay đổi hành vi CÓ CHỦ ĐÍCH (so với trạng thái hiện tại trong working tree)
| profile | fps | hiện tại | MỚI | ghi chú |
| --- | --- | --- | --- | --- |
| local_precise | 30/60/144 | 35001/17501/7293 | **33334/16667/6945** | revert 1.05→1.0 (giải REJECT) |
| audience_safe | 60 | 20001 | **17501** | mục tiêu chính của user (sắc hơn 2.5ms) |
| audience_safe | 144 | 18000 | **7293** | bỏ floor (rủi ro remote đã chấp nhận) |
| audience_safe | 30 | 40001 | **35001** | 1.05 frame |
| balanced | 144 | 14000 | **8334** | bỏ floor → đúng 1.2 frame |
| dense_safe | — | (tồn tại) | **XOÁ** | gộp; mọi nơi trỏ dense → còn-giữ |

Mọi ô @30/@60 của balanced và @None của cả 3 (trừ đã liệt kê) **giữ nguyên**.

---

## 2. Change set theo file

### 2.1 `src/sky_music/config.py`
- `DEFAULT_TIMING_PROFILES`: **xoá hẳn entry `dense_safe`**. Với 3 profile còn lại, xoá mọi
  `*_floor_us` (`hold_floor_us` đã xoá ở task trước; xoá nốt `min_hold_floor_us`). Đặt:
  - local_precise: `min_hold_frames: 1`, `min_hold_unframed_us: 22000`.
  - balanced: `min_hold_frames: 1.2`, `min_hold_unframed_us: 17000`.
  - audience_safe: `min_hold_frames: 1.05`, **thêm `min_hold_unframed_us: 18000`** (trước đây fps=None
    của audience lấy 18000 từ floor; floor mất nên phải khai báo tường minh để giữ @None=18000).
  - Giữ `spin_threshold_us`, `focus_restore_grace_us` mỗi profile.
- `CLI_PROFILE_NAMES`: bỏ `"dense-safe"`. `_PROFILE_KEY_TO_CLI`: bỏ `dense_safe`. `normalize_profile_name`:
  bỏ nhánh dense nếu có (không có nhánh đặc biệt — chỉ cần không map; canonical tự fallback balanced).

### 2.2 `src/sky_music/domain/scheduler_types.py`
- `materialise_frame_floor(frames, floor, frame_us)` → đổi thành thuần `ceil(frames × frame_us)` (bỏ
  tham số/`max` floor). Đổi tên gợi ý: `materialise_frame_us(frames, frame_us)`. Cập nhật 2 call site
  (hold, min_hold) bỏ tham số floor.
- `TimingPolicy`: xoá field `hold_floor_us`, `min_hold_floor_us`. `from_dict`: bỏ `floor_key`/`fallback_floor`
  khỏi `frame_coupled` (giờ chỉ còn frames + unframed + override). Giữ nguyên gate `declares_hold` →
  hold mirror min_hold.
- `FrameTimingPolicy`: xoá field `hold_floor_us`/`min_hold_floor_us` nếu có; nhánh fps>0 materialise
  bằng `ceil(frames×frame_us)`; nhánh override (`*_override_us`) + fps=None (`*_unframed`/raw) giữ.
- Xoá classmethod `TimingPolicy.dense_safe()` và `FrameTimingPolicy.dense_safe()`.

### 2.3 `src/sky_music/domain/validation.py`
- `_frame_coupled_us` / `_has_frame_model`: bỏ nhánh `*_floor_us`; frame-coupled = `ceil(frames×frame_us)`
  (mirror materialise mới, vẫn ceil frame period). `_hold_us`/`_min_hold_us` cập nhật theo.
- `validate_timing_profile`: xoá các check `hold_floor_us`/`min_hold_floor_us` (≥0, ordering). Giữ
  check `frames>0`, `min_hold_frames>=1.0`, `min<=hold` (frames), `min_hold_us > frame_us`,
  guard `<10000`. **Lưu ý:** local_precise 1.0 @60 → min_hold=16667 > frame_us 16666.67 (qua nhờ ceil) ✓.
- **Xoá hẳn** `validate_audience_safe_profile`, alias `validate_audience_safe_base_profile`,
  `validate_audience_safe_runtime_policy`, và nhánh `if normalized == "audience_safe"` trong
  `validate_builtin_timing_profile`. Audience giờ chỉ là frames=1.05, validate như profile thường.
- Grep gọi các hàm audience-validator đã xoá (main.py/session_context/tests) → gỡ.

### 2.4 `src/sky_music/domain/session_context.py`
- Set `profile_fields` (dòng ~139) bỏ `hold_floor_us`, `min_hold_floor_us`.

### 2.5 Re-point mọi gợi ý "dense-safe" → profile còn giữ
Logic: same-key infeasible cần `min_hold` NHỎ hơn ⇒ profile sắc nhất = **local_precise** (1.0). Profile
"an toàn dày" giờ là balanced/audience. Sửa:
- `scheduler.py:233` `recommended_profile="dense-safe"` → **`"local-precise"`** (kèm khuyến nghị giảm tempo
  vốn đã có).
- `analyzer.py:203` `"dense-safe" if has_repeats else "audience-safe"` → thay "dense-safe" bằng
  **"local-precise"** (repeats nhanh cần min_hold nhỏ).
- `calibration.py:96,106` mọi `"dense-safe"` → ánh xạ lại: stress/repeat → **"local-precise"**; còn lại
  giữ "audience-safe"/"balanced" theo nhánh hiện có. Giữ ý nghĩa "p99 cao → profile an toàn hơn".

### 2.6 UI / CLI text
- `main.py:802` help text: bỏ dòng "dense-safe (many chords/repeats)". `main.py` `frame_coupled_ms`
  (bảng so sánh): bỏ tham chiếu `*_floor_us` (giờ chỉ đọc frames/min_hold). `main.py:630` dùng
  `exc.recommended_profile` — tự đúng sau 2.5.
- `picker.py:139`: xoá entry `("dense-safe", …)`.

### 2.7 Khôi phục local_precise = 1.0 + revert test bị khoá 1.05 (giải REJECT)
- `config.py` local_precise `min_hold_frames: 1` (không phải 1.05).
- Revert giá trị test về baseline 1.0: `test_calibration.py` (`35_001`→`33_334`),
  `test_empirical_floors.py` (parametrize `35001/17501/7293`→`33334/16667/6945`; `local.hold_us`
  `7293`→`6945`), `test_scheduler_new.py` (`policy.hold_us/min_hold_us 35_001`→`33334`),
  `test_session_context.py` (comment "1.05 frames"→"1.0 frame").

### 2.8 Docs (cập nhật, đánh dấu superseded — KHÔNG xoá lịch sử)
- `timing-principles.md`, `timing-profile-frame-model.md`, `architecture.md`: ghi rõ **floor đã bị xoá**,
  mô hình giờ thuần frame-relative `ceil(frames×frame_us)`; **chỉ còn 3 profile**; audience = 1.05 frame
  (sắc, KHÔNG còn tường remote tuyệt đối) + caveat rủi ro remote FPS cao. Có thể thêm
  `docs/floor-removal-three-profile-plan.md` (file này) vào mục "Liên quan".

---

## 3. Cổng nghiệm thu cơ học (reviewer dùng)
1. **Bảng baseline MỚI §1** khớp từng dòng cho 3 profile × {None,30,60,144}; hold==min mọi dòng.
2. **Intended delta đúng & chỉ đúng các ô §1** (audience 60/144/30, balanced 144, local revert). Không
   ô nào ngoài danh sách đổi.
3. **dense_safe biến mất hoàn toàn:** `grep -rniE "dense[_-]safe" src/` rỗng (trừ docs lịch sử có chú
   thích). `--timing-profile dense-safe` → argparse báo invalid choice. `from_profile_name("dense_safe")`
   → canonical fallback balanced (không raise).
4. **floor biến mất:** `grep -rniE "min_hold_floor_us|hold_floor_us|materialise_frame_floor|validate_audience" src/`
   rỗng. `FrameTimingPolicy` không còn field floor.
5. **Escape hatch còn sống:** `from_dict({'min_hold_us':1000,'hold_us':2000})`@None → (2000,1000);
   `--hold-ms 24 --min-hold-ms 10` → policy hold≠min, band nén "moderate" khi interval∈[min,hold).
6. **Golden xanh không regenerate:** `pytest tests/test_golden_regression.py -q`.
7. **Full suite xanh:** `pytest -q` (số test giảm do xoá test dense/floor/audience-validator — hợp lệ,
   KHÔNG được fail).
8. **Recommendations hợp lệ:** strict-mode repeat infeasible → `recommended_profile == "local-precise"`;
   analyzer/calibration không trả "dense-safe".
9. **min_hold an toàn:** mọi profile @{30,60,144} có `min_hold_us > frame_us` và ≥1 frame; @60 không
   tụt xuống dưới ~16.7ms cho local (16667), audience (17501), balanced (20001).

---

## 4. Quy tắc thực thi
- Tách commit theo cụm: (a) revert local 1.0 + test, (b) xoá floor (model+validation+session+main),
  (c) xoá dense_safe + re-point recommendations + UI/CLI, (d) docs. `pytest` sau mỗi cụm.
- KHÔNG đụng `plan_same_key_hold`, golden, escape hatch, hay giá trị @30/@60 ngoài danh sách §1.
- Test "phải giữ xanh" (escape hatch/override/golden/band) mà đỏ ⇒ DỪNG, báo reviewer — dấu hiệu
  derive/materialise rò sang đường escape hatch.
- KHÔNG tự thêm lại floor dưới tên khác. Nếu thấy một cổng không pass được mà không đổi behavior ngoài
  §1 ⇒ DỪNG, báo lại.

## 5. Liên quan
- `docs/hold-min-hold-unification-plan.md` (task trước: hold derive = min)
- `docs/timing-architecture-audit.md` §6 (phong cách handoff)
- `docs/timing-principles.md` Appendix A.9 / EXP-4 (floor audience chưa chứng minh cần — cơ sở cho việc bỏ)

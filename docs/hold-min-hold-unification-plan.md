# Hold/Min-Hold Unification Plan (Direction B) — Handoff Contract

Date: 2026-06-04
Author: planning AI · Executor: another AI · Final reviewer: planning AI

> **Mục tiêu (quyết định của user — Hướng B).** Ở đường **production frame-aware**, `hold_us` luôn
> bằng `min_hold_us` (đã chứng minh: mọi built-in khai báo cặp `hold_*` y hệt cặp `min_hold_*`). Đây
> là **dư thừa giá trị + khai báo trùng** trong mọi built-in. Hướng B: **đưa `min_hold` thành nguồn
> sự thật DUY NHẤT cho built-in**, `hold` chỉ còn tồn tại như **escape hatch raw/override** (fps=None
> và CLI `--hold-ms` / config `hold_us`). Band nén (`plan_same_key_hold`) **giữ nguyên cơ chế**, chỉ
> còn kích hoạt khi override đặt `hold > min`. KHÔNG tái lập band nén cho built-in (Option C đã bị
> audit bác).

> **Phạm vi đã chốt:** "cho phép đổi nội bộ rộng". KHÔNG đổi CLI behavior quan sát được ngoài 2 delta
> cố ý ở §3. KHÔNG xoá `hold_us` khỏi data model / scheduler / CLI — chỉ xoá **khai báo trùng khỏi
> built-in** và thêm luật **derive `hold := min_hold` khi profile không khai báo `hold`**.

---

## 0. Nguyên tắc bất biến (đọc trước khi sửa)

1. **Production frame-aware (fps>0) BẤT BIẾN tuyệt đối.** Mọi giá trị materialise hold/min_hold của
   cả 4 profile ở fps ∈ {30,60,144} phải **y hệt baseline §2**. (Vì built-in vốn đã hold==min ở
   fps>0, derive `hold:=min` cho ra số y hệt.)
2. **Escape hatch còn sống.** Khi profile/override/CLI khai báo `hold_us` (hoặc `hold_frames`/
   `hold_floor_us`/`hold_unframed_us`) tường minh → giá trị đó được tôn trọng độc lập với min_hold
   (cho thí nghiệm đo sàn in-game theo Appendix A). `plan_same_key_hold` **không đổi một dòng**.
3. **Band nén chỉ qua override.** Built-in sau refactor có hold==min ⇒ band rỗng (đúng như hiện
   nay). Override đặt hold>min ⇒ band hoạt động (risk "moderate"/compressed) — phải vẫn chạy.
4. **Golden schedules BẤT BIẾN.** `tests/test_golden_regression.py` xanh không cần regenerate.

---

## 1. Hiện trạng (gốc rễ dư thừa)

`config.py::DEFAULT_TIMING_PROFILES` — mỗi built-in khai báo **cả hai** cụm, trùng nhau ở phần
frame-aware:

| profile | hold_frames / hold_floor_us / hold_unframed_us | min_hold_frames / min_hold_floor_us / min_hold_unframed_us |
| --- | --- | --- |
| local_precise | 1 / 0 / 22000 | 1 / 0 / 22000 |
| balanced | 1.2 / 14000 / 26000 | 1.2 / 14000 / 17000 |
| dense_safe | 1.2 / 11000 / 22000 | 1.2 / 11000 / 17000 |
| audience_safe | 1.2 / 18000 / (—) | 1.2 / 18000 / (—) |

Cặp `frames`/`floor` của hold **giống hệt** min_hold ở cả 4 ⇒ ở fps>0 hold==min. Chỉ `hold_unframed_us`
của balanced (26000) và dense (22000) là khác `min_hold_unframed_us` (17000) — và **chỉ lộ ra ở
fps=None** (escape hatch).

Nơi `policy.hold_us` thực sự tác động lịch: **chỉ** `scheduler.py:207` (cận trên band nén). Còn lại
là hiển thị/ước lượng: `source_duration_us` (`scheduler.py:316`), calibration recommend
(`calibration.py:133`), HUD/table (`main.py:340,427,483`).

---

## 2. Baseline đông cứng (chụp trước refactor)

Lệnh tái lập (equivalence-check mọi bước):

```
uv run python -c "
from sky_music.domain.scheduler_types import FrameTimingPolicy
for name in ['local_precise','balanced','dense_safe','audience_safe']:
    for fps in (None,30,60,144):
        p=getattr(FrameTimingPolicy,name)(fps=fps)
        print(name,fps,p.hold_us,p.min_hold_us)
"
```

Baseline hiện tại (hold, min_hold):

```
local_precise None 22000 22000      local_precise 30 33334 33334   60 16667 16667   144 6945 6945
balanced      None 26000 17000      balanced      30 40001 40001   60 20001 20001   144 14000 14000
dense_safe    None 22000 17000      dense_safe    30 40001 40001   60 20001 20001   144 11000 11000
audience_safe None 18000 18000      audience_safe 30 40001 40001   60 20001 20001   144 18000 18000
```

Test suite trước refactor: **212 passed** (`uv run python -m pytest -q`).

---

## 3. Thay đổi CÓ CHỦ ĐÍCH (chỉ đúng 2 ô đổi)

Sau refactor, derive `hold:=min_hold` cho built-in ⇒ **chỉ** `hold_us` ở **fps=None** của balanced &
dense đổi (band nén raw collapse — user đã chấp nhận):

| profile | fps | hold_us cũ | hold_us MỚI | min_hold (không đổi) |
| --- | --- | --- | --- | --- |
| balanced | None | 26000 | **17000** | 17000 |
| dense_safe | None | 22000 | **17000** | 17000 |

**MỌI ô khác BẤT BIẾN** (cả 4 profile × {30,60,144}, và local/audience ở None).

---

## 4. Change set chi tiết theo file

### 4.1 `src/sky_music/config.py` — xoá khai báo trùng khỏi built-in
- Xoá `hold_frames`, `hold_floor_us`, `hold_unframed_us` khỏi **cả 4** profile trong
  `DEFAULT_TIMING_PROFILES`. Giữ nguyên toàn bộ cụm `min_hold_*`, `spin_threshold_us`,
  `focus_restore_grace_us`.
- Thêm comment: "hold cố ý KHÔNG khai báo — derive = min_hold (xem hold-min-hold-unification-plan.md).
  Đặt `hold_*` tường minh chỉ khi cần escape hatch/thí nghiệm."

### 4.2 `src/sky_music/domain/scheduler_types.py` — luật derive trong `TimingPolicy.from_dict`
- **Luật:** `declares_hold = any(k in p_dict for k in ("hold_us","hold_frames","hold_floor_us","hold_unframed_us"))`.
  - `declares_hold == True` → tính `hold` qua `frame_coupled(...)` **y như hiện tại** (escape hatch).
  - `declares_hold == False` → **mirror min_hold**: `hold_us=min_hold_us`, `hold_frames=min_hold_frames`,
    `hold_floor_us=min_hold_floor_us`, `hold_override_us=min_hold_override_us`,
    `hold_uses_frame_model=min_hold_uses_frame_model`.
- **Sharp edge — `base` fallback:** `frame_coupled` đang fallback về `base = DEFAULT_TIMING_PROFILES
  ["balanced"]`. Sau khi balanced mất các key `hold_*`, fallback của nhánh hold (chỉ chạy khi
  `declares_hold`) sẽ đổi. An toàn nhất: tính `min_hold` **trước**, rồi nhánh hold-mirror dùng thẳng
  giá trị min_hold đã tính (không đụng `base`). Nhánh `declares_hold` vẫn dùng `frame_coupled` cũ.
  Kiểm bằng cổng equivalence §5.

### 4.3 `src/sky_music/domain/validation.py` — mirror derive ở các validator đọc `hold_floor_us`
- `validate_audience_safe_profile` (dòng 282–294) đọc `profile.get("hold_floor_us", profile.get(
  "hold_us", 0))`. Sau khi audience_safe mất `hold_floor_us`, biểu thức này → 0 → **raise sai**. Sửa
  để derive: khi profile **không** có bất kỳ key `hold_*` nào → coi `hold_floor_us := min_hold_floor_us`
  và `hold_us(effective) := min_hold_us`. (Audience min_hold_floor=18000 ⇒ vẫn pass ≥18000.)
- `_hold_us` (dòng 199–208): sau khi built-in mất key hold, `_hold_us` trả `None` ⇒ `validate_hold_
  ordering` bỏ qua check hold (min-only) — **đúng và an toàn** (ordering hold==min hiển nhiên đạt).
  Xác nhận `validate_timing_profile`/`validate_builtin_timing_profile` vẫn pass cho cả 4 built-in.

### 4.4 `src/sky_music/main.py` — bảng so sánh profile
- `_print_profile_comparison_table` → `frame_coupled_ms(d, value_key="hold_us", floor_key="hold_floor_us")`
  (dòng 340). Sau khi built-in mất key hold → in "0". Sửa `frame_coupled_ms` (hoặc cột hold) để
  fallback sang `min_hold_floor_us`/`min_hold_us` khi không có key hold. Cột `min_hold_ms` giữ nguyên.
- Plumbing `--hold-ms` (dòng 340 trong `policy_overrides`, và `session_context.py:78`) **không đổi** —
  escape hatch.

### 4.5 KHÔNG đổi
- `scheduler.py` `plan_same_key_hold` + nhánh band nén — **giữ nguyên** (bất biến §0.2/§0.3).
- `calibration.py:133` `recommended_hold = effective.hold_us` — không đổi code; giá trị tự đúng vì
  `effective.hold_us` đã derive (= min ở built-in). Ở balanced fps=None nó đổi 26000→17000 (đúng §3).
- CLI flags, config schema, `FrameTimingPolicy` shape — không đổi.

---

## 5. Cổng nghiệm thu cơ học (final reviewer dùng)

1. **Equivalence fps>0 (BẤT BIẾN tuyệt đối):** chạy lệnh §2, so từng dòng. Mọi profile × {30,60,144}
   **y hệt baseline**; local_precise & audience_safe ở None cũng y hệt (22000/22000, 18000/18000).
2. **Intended delta đúng & duy nhất:** chỉ `balanced None hold 26000→17000` và `dense_safe None
   22000→17000`. Không ô nào khác đổi.
3. **Escape hatch còn sống:**
   - `TimingPolicy.from_dict({"min_hold_us":1000,"hold_us":2000})` @fps=None → `hold_us==2000`,
     `min_hold_us==1000` (band raw còn).
   - `TimingPolicy.from_dict({"min_hold_us":1000,"hold_us":100000})` @fps=60 → `min_hold_us==20834`
     (override hold không kéo min lên/xuống sai).
   - CLI `--hold-ms 24 --min-hold-ms 10` qua `PlaybackSessionContext.from_cli_args` → policy có
     hold≠min và `plan_same_key_hold` cho ra nhánh "moderate" khi interval ∈ [min,hold).
4. **Band nén qua override còn chạy:** test `plan_same_key_hold(target=26000,min=17000,delta=21000)`
   → `risk=="moderate"`, `hold==21000`, `compressed`.
5. **Validator built-in pass:** `validate_builtin_timing_profile("audience-safe", <dict đã bỏ hold>,
   selected_fps=60)` không raise; cả 4 built-in qua `validate_timing_profile`.
6. **Golden bất biến:** `uv run python -m pytest tests/test_golden_regression.py -q` xanh, không
   regenerate.
7. **Full suite xanh:** `uv run python -m pytest -q`. Số test có thể đổi do cập nhật các test ghim
   2 delta cố ý (§6) — KHÔNG được có test fail.
8. **Grep sạch:** `config.py` không còn `hold_frames|hold_floor_us|hold_unframed_us` ở 4 built-in
   (chỉ còn nếu user tự thêm trong config.json runtime — không tính).

---

## 6. Test phải cập nhật (thay đổi có chủ đích) vs phải giữ xanh

**Phải đổi (ghim giá trị fps=None của balanced/dense — đổi 26000/22000 → 17000):**
- `tests/test_empirical_floors.py:108-109` (balanced None hold 26000→17000).
- `tests/test_calibration.py:24-25` (balanced unframed hold 26000→17000); kiểm thêm `:235`.
- `tests/test_scheduler_new.py:425-426` (balanced None hold 26000→17000).
- Bất kỳ assert nào khác đọc balanced/dense hold ở fps=None/unframed (grep `26000`, `22000` trong
  ngữ cảnh hold built-in).

**Phải GIỮ XANH không sửa (escape hatch / override / fps>0):**
- `tests/test_empirical_floors.py:36-50` (explicit hold_us override) — còn sống.
- `tests/test_acceptance_flow.py:69` (`p_bal.hold_us != p_dense.hold_us` @fps=120) — vẫn khác **vì
  min_hold_floor khác** (14000 vs 11000) → 14000 vs 11000. KHÔNG được làm test này đổi.
- `tests/test_scheduler_new.py` các test `plan_same_key_hold`/band với hold≠min tường minh
  (39-56,120,145-173,211,351,442,528-533).
- `tests/test_calibration.py:128-144` (override/calibration hold≠min).
- Golden + local_precise/audience ở mọi fps.

> ⚠️ Nếu phát hiện một test "phải giữ xanh" lại đỏ sau thay đổi → **DỪNG, báo lại reviewer**; đó là
> dấu hiệu derive làm rò rỉ sang đường escape hatch/fps>0, KHÔNG được nới assert để né.

---

## 7. Quy tắc cho bên thực thi
- Một commit duy nhất (hoặc tách: config+derive | validation | main-display | tests), chạy `pytest`
  sau mỗi bước.
- KHÔNG đụng `plan_same_key_hold`, CLI flags, golden, hay bất kỳ giá trị fps>0 nào.
- Field lạ trong `config.json` của user: vẫn graceful (cơ chế `merged_timing_profiles` sẵn có) — nếu
  user còn `hold_*` override thì phải vẫn được tôn trọng (escape hatch).
- Nếu một cổng §5 không thể pass mà không đổi behavior ngoài 2 delta §3 → DỪNG, báo lại.

## 8. Liên quan
- `docs/timing-architecture-audit.md` (§6 handoff style; Option A repeat-gap removal)
- `docs/scheduler-core-architecture-plan.md` (Option A/C; vì sao không tái lập band built-in)
- `docs/timing-principles.md` Appendix A.3 (hold = 1 frame, giữ lâu hơn vô ích cho onset đơn)

# Docs Cleanup Plan — dọn `docs/` về một nguồn sự thật

> Status: PROPOSED (chưa thực thi). Mục tiêu: gỡ thông tin sai/cũ/trùng trong `docs/`, gom về một
> tập canonical nhỏ, neo mọi thứ vào **một chân lý nền** và **bằng chứng đã kiểm chứng**.
> Phạm vi: **CHỈ tài liệu.** Không đổi code, không đổi hành vi playback.

---

## 0. Chân lý nền (ground truth) — mọi doc phải nhất quán với điều này

**Game nhận input theo FRAME.** Game lấy mẫu trạng thái phím mỗi khung hình; để một key-down được
ghi nhận, nó phải được giữ **≥ 1 frame thật của game**. Đây là ràng buộc timing **cứng duy nhất**.
Mọi quy tắc timing khác chỉ là hệ quả hoặc phương tiện phục vụ ràng buộc này.

### Thứ bậc nguồn sự thật (evidence hierarchy) — dùng để xử mọi mâu thuẫn giữa các doc
1. **Hành vi game quan sát được** (âm thanh/onset thu trong game) — thắng tất cả.
2. **Đo deterministic từ code** (mô phỏng coordinator/scheduler thật, telemetry) — thắng "kinh nghiệm".
3. **Code hiện hành** (`src/`) — thắng mọi mô tả trong doc.
4. **Doc** — chỉ là diễn giải; nếu lệch (1)/(2)/(3) thì doc SAI và phải sửa.

> Quy tắc vàng: nếu một câu trong doc không truy được về (1), (2), hoặc (3), nó là **nợ kỹ thuật** và
> phải bị xoá hoặc gắn cờ.

---

## 1. Sự thật đã kiểm chứng trong phiên điều tra 2026-06-06 (nội dung canonical phải phản ánh)

Các kết luận này có số liệu/kiểm chứng kèm theo (xem `tests/measure_conflict_matrix.py`,
`tests/repro_roundtrip.py`) và PHẢI là xương sống của doc canonical mới:

1. **Sender/dispatch là deterministic và sạch.** 88 bài thật × 3 profile × 6 mức fps × 3 mức
   send_duration → **0 note bị rớt phía sender** (`dropped_conflict`). Mọi drop trong đo đạc đến từ
   bài synthetic `TEST_repeat_floor_*` (cố tình viết dưới sàn frame).
2. **Bài thật không bao giờ chạm sàn same-key.** Interval same-key nhỏ nhất toàn corpus = **76ms**
   (bài `blue`), p50 ≈ 996ms; 0 transition < 70ms. Sàn `min_hold` lớn nhất chỉ ~33ms (@30fps).
   ⟹ cơ chế same-key/hold-floor **không** là nguyên nhân mất note ở nhạc thật.
3. **Round-trip profile/fps cho policy y hệt.** local_precise@144 → đổi đi → quay lại = **6945µs ở
   mọi kịch bản** (picker persist, calibration persist, reload đĩa). Config trên đĩa **vô can**.
4. **Mất note thực tế là phía GAME (lấy mẫu theo frame) và/hoặc trạng thái runtime bay hơi**, không
   phải lỗi toán của player. **Toggle FPS KHÔNG phải cách sửa đáng tin.**
5. **`release_latency_margin_us` đã bị GỠ khỏi code** (2026-06-06). Sàn feasibility same-key giờ là
   **đúng `min_hold`**, không cộng biên cố định. Runtime tự xử độ trễ dispatch qua completion-anchor.
6. **Completion-anchor là contract hiện hành**: `release_not_before_us = down_dispatch_completed_us +
   min_hold_us` ⟹ observed hold ≥ 1 frame mỗi note.
7. **Chuẩn 1-frame của `local_precise` được GIỮ** (min_hold_frames = 1.0, zero margin) theo quyết định
   chủ dự án. Không quay lại 1.05/round().
8. **Hardening đường input đã thêm** (rủi ro thấp, ngoài timing): re-acquire cửa sổ game tươi mỗi lần
   play, guard timer 1ms trong dispatch thread, diagnostics play-start sau `PLAYBACK_DEBUG`.

---

## 2. Chẩn đoán hiện trạng `docs/` (vì sao cần dọn)

- **17 file, ~5.500 dòng**, phần lớn là doc "plan/audit/experiment" lịch sử chồng lớp, tham chiếu vòng.
- **Thông tin đã SAI so với code hiện tại:**
  - 6 doc còn mô tả `release_latency_margin_us` như cơ chế sống (`architecture.md`,
    `completion-anchor-refactor-plan.md`, `playback-flow-hardening-plan.md`,
    `runtime-hold-refactor-plan.md`, `timing-experiments.md`, `timing-principles.md`) — **đã gỡ khỏi code.**
  - `completion-anchor-refactor-plan.md` "locked decision" ghi *margin = fixed 500µs* — nay trái code.
  - 5 doc còn vương nội dung **start-anchor** (đã bị completion-anchor thay).
  - 5 doc còn nhắc các knob đã xoá (`input_lead`/`chord_merge`/`frame_align`/`release_gap`/
    `repeat_release_gap`) — đã xử lý bằng banner "RETIRED" nhưng vẫn gây nhiễu đọc.
- **Tham chiếu gãy:** nhiều doc trỏ `result.md` — **file không tồn tại.**
- **Trùng lặp:** mô hình frame được mô tả lại ở ≥4 doc với mức cập nhật khác nhau.
- **Không có doc canonical nào ghi sự thật mục 1** (đang nằm rải rác trong note điều tra untracked).

---

## 3. Kiến trúc đích cho `docs/` (target end-state)

Tách rõ **3 lớp doc** + **1 kho lưu trữ**:

```
docs/
  INDEX.md                       (BẢN ĐỒ: file nào canonical, file nào lịch sử, đọc gì trước)
  timing-principles.md           (CANONICAL — chân lý timing: frame model, hold/min_hold,
                                  completion-anchor, KHÔNG còn margin, evidence hierarchy)
  architecture.md                (CANONICAL — 4 lớp + pipeline play/input + hardening)
  timing-profile-frame-model.md  (REFERENCE — công thức frame, ngắn, trỏ về principles)
  timing-experiments.md          (EVIDENCE — chỉ giữ thí nghiệm CÒN MỞ + kết quả đã chốt)
  archive/                        (LỊCH SỬ — plan/audit đã hoàn thành/bị thay; read-only, có stamp)
    2026-06_completion-anchor-refactor-plan.md
    2026-06_runtime-hold-refactor-plan.md
    2026-06_timing-architecture-audit.md
    2026-06_floor-removal-three-profile-plan.md
    2026-06_hold-min-hold-unification-plan.md
    2026-06_down-hold-up-scheduling-audit.md
    2026-06_scheduler-core-architecture-plan.md
    2026-06_realtime-sender-thread-refactor-plan.md
    2026-06_play-input-architecture-refactor-plan.md
    2026-06_playback-flow-hardening-plan.md
    2026-06_playback-input-investigation-2026-06-06.md
    2026-06_timing-guard-binding-audit.md
    2026-06_ui-overhaul-textual-plan.md
```

Nguyên tắc: **AGENTS.md vẫn là single source of truth cho RULES dự án** (không gộp vào timing).
`docs/` chỉ mô tả kiến trúc + timing + bằng chứng. `README.md` giữ nguyên vai trò user-facing.

### Ngôn ngữ (chuẩn hoá)
- **Tất cả doc canonical → tiếng Anh** (`timing-principles.md`, `architecture.md`,
  `timing-profile-frame-model.md`, `INDEX.md`).
- **`timing-experiments.md` → tiếng Anh**, NGOẠI TRỪ **nội dung chi tiết của đúng 1 mục được giữ
  tiếng Việt** (mục diễn giải/biên bản đo chi tiết mà user muốn đọc bằng tiếng Việt; đánh dấu rõ
  bằng heading để không lẫn). Phần khung/protocol còn lại dịch tiếng Anh.
- `archive/` giữ nguyên ngôn ngữ gốc (không dịch lại lịch sử).

---

## 4. Bảng xử lý từng file (disposition)

| File | Dòng | Xử lý | Lý do |
|---|---|---|---|
| `timing-principles.md` | 756 | **REWRITE (canonical)** | Là SSOT timing nhưng còn margin/start-anchor + banner knob-retired dày. Cắt phần RETIRED về phụ lục ngắn, sửa §7 theo completion-anchor + bỏ margin, nhúng sự thật mục 1. |
| `architecture.md` | 93 | **REWRITE (canonical)** | Sửa dòng "min_hold + release_latency_margin", bổ sung pipeline play/input mới + hardening. |
| `timing-profile-frame-model.md` | 78 | **KEEP + edit nhẹ** | Công thức frame đúng; chỉ trỏ về principles, xác nhận "no margin". |
| `timing-experiments.md` | 764 | **PRUNE mạnh** | **CHỈ giữ 2 thí nghiệm: `spin_threshold_us` (O10.5) và `focus_restore_grace_us` (O10.6)** — đây là hai tham số CHƯA được kiểm duyệt về sự tồn tại/tác dụng. Mọi thí nghiệm khác (O1–O10.4, Part 1 floors đã chốt, lịch sử start-anchor/margin) → cắt/đưa archive. Dịch sang tiếng Anh, **trừ phần nội dung chi tiết của đúng 1 mục giữ tiếng Việt** (xem §3 Ngôn ngữ). |
| `completion-anchor-refactor-plan.md` | 204 | **ARCHIVE + correction stamp** | Đã implement; nhưng "locked: margin=500µs" nay TRÁI code → stamp nói rõ margin đã gỡ. |
| `runtime-hold-refactor-plan.md` | 963 | **ARCHIVE** | Đã có "Superseded anchor note"; lịch sử. |
| `timing-architecture-audit.md` | 275 | **ARCHIVE** | Đã có "SUPERSEDED note"; lịch sử audit knob. |
| `floor-removal-three-profile-plan.md` | 171 | **ARCHIVE** | Quyết định đã thực thi (frame model live). |
| `hold-min-hold-unification-plan.md` | 202 | **ARCHIVE** | hold suy ra từ min_hold đã là code hiện hành. |
| `down-hold-up-scheduling-audit.md` | 365 | **ARCHIVE** | Audit lịch sử trước completion-anchor. |
| `scheduler-core-architecture-plan.md` | 255 | **ARCHIVE** | Plan đã hấp thụ vào scheduler hiện tại. |
| `realtime-sender-thread-refactor-plan.md` | 188 | **ARCHIVE** | Đã implement (`infrastructure/realtime.py` tồn tại). |
| `play-input-architecture-refactor-plan.md` | 226 | **ARCHIVE** | Plan untracked; nội dung đã/đang phản ánh ở code + canonical. |
| `playback-flow-hardening-plan.md` | 306 | **ARCHIVE** (rút phần "đã làm" vào architecture) | Hardening đã thực hiện phần lõi; còn nhắc margin. |
| `playback-input-investigation-2026-06-06.md` | 108 | **MERGE → archive** | Kết luận đúng → đưa vào mục "Investigation findings" của principles, rồi archive bản gốc. |
| `timing-guard-binding-audit.md` | 116 | **ARCHIVE** | Audit binding cũ; nhắc knob đã xoá. |
| `ui-overhaul-textual-plan.md` | 409 | **ARCHIVE** | UI Textual đã LIVE (đang dùng). Kế hoạch còn lại là **xoá hẳn UI classic cũ** — đó là việc CODE (ngoài phạm vi dọn docs); ghi 1 dòng "classic-UI removal pending" trong INDEX. |
| `result.md` | — | **N/A** | Không tồn tại — gỡ mọi link tới nó. |

---

## 5. Danh sách lỗi cụ thể phải xoá/sửa (purge list)

Mỗi mục dưới đây là một grep-target; sau dọn, các cụm này KHÔNG được còn trong **doc canonical**
(được phép tồn tại trong `archive/` như lịch sử, nhưng phải nằm dưới banner stamp):

1. `release_latency_margin_us` / "min_hold + release_latency_margin" → thay bằng "feasibility floor =
   `min_hold`" (đã gỡ khỏi code).
2. `dispatch_started_us + min_hold` / "start-anchor" như contract HIỆN HÀNH → thay bằng
   completion-anchor (`dispatch_completed_us + min_hold`).
3. "~50% of notes land below 1 frame" như tình trạng HIỆN TẠI → đó là tình trạng TRƯỚC fix; nêu rõ
   completion-anchor đã giải quyết.
4. Knob đã xoá (`input_lead`, `chord_merge`, `frame_align`, `release_gap`, `repeat_release_gap`) →
   chỉ còn 1 dòng tóm tắt "đã gỡ, xem archive", không lặp lại đặc tả.
5. Link tới `result.md` → xoá hoặc thay bằng số liệu nội tuyến/`tests/measure_conflict_matrix.py`.
6. "Toggle FPS để sửa mất note" như workaround sản phẩm → ghi rõ KHÔNG đáng tin (mục 1.4).
7. Mọi tuyên bố "player mất note do X toán học" mà không truy được về evidence → xoá; thay bằng
   "sender sạch; mất note là game-side/volatile".

---

## 6. Nội dung doc canonical phải có (content spec)

### `timing-principles.md` (sau rewrite) — outline đề xuất
1. Chân lý nền: game nhận theo frame (mục 0).
2. Mô hình hold: `hold = ceil(frames × ceil(1e6/fps))`; `local_precise = 1.0 frame` (zero margin, đã
   validate in-game, giữ nguyên). `min_hold` = sàn visibility.
3. Same-key feasibility: khả thi ⟺ `interval ≥ min_hold`. Dưới ngưỡng: strict từ chối, degraded giữ
   min_hold và báo overlap. **Không có margin.**
4. Completion-anchor: định nghĩa + vì sao (game quan sát inject-to-inject).
5. Evidence hierarchy (mục 0).
6. Investigation findings 2026-06-06 (mục 1: sender sạch, round-trip bất biến, game-side/volatile).
7. Phụ lục "RETIRED knobs" — 1 đoạn ngắn + trỏ archive (không giữ §9/§10/§11/§15 dài).

### `architecture.md` (sau rewrite) — bổ sung
- Pipeline play/input cập nhật: scheduler → `compile_runtime_intents` → `RuntimeDispatchCoordinator`
  → dispatch thread (MMCSS + waitable timer + timer-guard) → backend SendInput.
- Mục "Robustness/hardening": per-play window re-acquire, 1ms timer guard, diagnostics.
- Sửa mô tả same-key (bỏ margin).

---

## 7. Trình tự thực thi (phased, an toàn, mỗi phase 1 commit)

> Mỗi phase tự đứng được và không phá link. Dùng `git mv` để giữ lịch sử khi archive.

- **Phase 0 — Khung & index.** Tạo `docs/archive/`. Tạo `docs/INDEX.md` (bản đồ + thứ bậc sự thật).
  Chưa di chuyển gì. Commit.
- **Phase 1 — Canonical truth.** Rewrite `timing-principles.md` (nhúng mục 0/1, bỏ margin/start-anchor,
  rút RETIRED về phụ lục). Rewrite `architecture.md`. Edit nhẹ `timing-profile-frame-model.md`. Commit.
- **Phase 2 — Evidence prune.** Cắt `timing-experiments.md` về phần còn mở + kết quả chốt; sửa margin/
  start-anchor; gom lịch sử vào 1 mục. Commit.
- **Phase 3 — Archive plans/audits.** `git mv` 12–13 file plan/audit vào `docs/archive/` với tiền tố
  `2026-06_`; thêm **stamp 3 dòng** đầu mỗi file (xem §8). Fold kết luận của
  `playback-input-investigation-2026-06-06.md` vào principles §6 trước khi archive. Commit.
- **Phase 4 — UI doc & link hygiene.** Quyết định KEEP/ARCHIVE `ui-overhaul-textual-plan.md`. Sửa toàn
  bộ link gãy (`result.md`, link tới file đã archive → trỏ `archive/...`). Cập nhật con trỏ trong
  `CLAUDE.md` nếu cần (vẫn trỏ AGENTS.md). Commit.
- **Phase 5 — Validation gate.** Chạy checklist §9. Sửa nốt. Commit cuối + cập nhật `INDEX.md`.

---

## 8. Stamp chuẩn cho file archive (dán 3 dòng đầu file, không sửa nội dung cũ bên dưới)

```markdown
> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: <vd: file này nhắc release_latency_margin_us — đã GỠ khỏi code>.
```

---

## 9. Tiêu chí nghiệm thu (acceptance / guardrails)

Dọn xong khi TẤT CẢ đúng:

- [ ] `grep -rn "release_latency_margin" docs/ --include='*.md'` → chỉ còn trong `docs/archive/` (dưới stamp).
- [ ] `grep -rln "start-anchor\|dispatch_started_us + min_hold" docs/` không có hit ngoài `archive/`.
- [ ] `grep -rln "input_lead\|chord_merge\|frame_align\|repeat_release_gap" docs/` không hit ngoài
      `archive/` + 1 đoạn tóm tắt RETIRED trong principles.
- [ ] Không còn link tới `result.md` ở bất kỳ đâu.
- [ ] Mọi link tương đối trong doc canonical resolve được (không trỏ file đã move mà chưa cập nhật).
- [ ] `timing-principles.md` và `architecture.md` đều nêu rõ: game nhận theo frame; feasibility floor =
      min_hold (no margin); completion-anchor; sender sạch/round-trip bất biến; FPS-toggle không phải fix.
- [ ] `docs/INDEX.md` liệt kê đúng file canonical vs archive.
- [ ] Không file canonical nào mâu thuẫn nhau về margin/anchor/1-frame.
- [ ] `git mv` được dùng (lịch sử file được giữ), không xoá trắng.

---

## 10. Rủi ro & non-goals

**Rủi ro:**
- Link nội bộ gãy khi archive → giảm thiểu bằng Phase 4 + checklist link.
- Mất ngữ cảnh lịch sử nếu xoá thay vì archive → **luôn archive, không delete** (trừ `result.md` vốn không tồn tại).
- Memory/`MEMORY.md` của trợ lý trỏ tên file cũ → sau khi move, cập nhật con trỏ nếu có (kiểm
  `[[...]]`/đường dẫn trong memory liên quan timing).

**Non-goals (KHÔNG làm trong đợt này):**
- Không sửa code/hành vi playback.
- Không đổi `AGENTS.md` rules.
- Không viết lại `README.md` (chỉ rà link nếu trỏ docs đã move).
- Không quyết định lại các vấn đề timing đã chốt (1-frame, completion-anchor, bỏ margin).

---

## 11. Quyết định đã chốt (2026-06-06)
1. **UI:** Textual đã LIVE → `ui-overhaul-textual-plan.md` ARCHIVE. Việc xoá hẳn UI classic là task
   code riêng (ngoài phạm vi dọn docs), ghi nhận "pending" trong INDEX.
2. **Experiments:** `timing-experiments.md` chỉ giữ 2 thí nghiệm cho `spin_threshold_us` và
   `focus_restore_grace_us` (hai tham số chưa kiểm duyệt). Phần còn lại cắt/archive.
3. **Ngôn ngữ:** canonical → tiếng Anh; experiments → tiếng Anh trừ đúng 1 mục chi tiết giữ tiếng Việt.

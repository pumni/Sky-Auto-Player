# Intra-chord micro-stagger — chống mất nốt ở người nghe từ xa

> **Ngày:** 2026-06-27
> **Phạm vi:** Đường gửi hợp âm (`build_key_actions` → coordinator → dispatch → `SendInput`).
> **Triệu chứng cần giải:** local nghe đủ nốt, nhưng **người nghe từ xa** thỉnh thoảng **mất nốt**, nặng nhất ở **hợp âm nhiều phím**; nhịp đến vẫn đúng.

## 1. Chẩn đoán (vì sao remote mất, local không)

- Pipeline local đã gần tối ưu tuyệt đối và đã được chứng minh hai lần:
  `docs/hold-timbre-rhythm-findings.md` (down-lateness phẳng, p99 < 100µs) và
  `docs/chord-dispatch-review-2026-06-26.md` §2 (một hợp âm = **một** `SendInput` nguyên khối +
  adaptive-lead bucket-theo-polyphony). Đây là mức đồng thời tối đa Windows cho phép.
- Vì **local đủ mà remote thiếu**, nguyên nhân **không** nằm ở timing dispatch local. Game vẽ nốt
  local trực tiếp từ hàng đợi input thô (luôn bắt đủ mọi key-down edge), còn nốt phát cho người khác
  đi qua một **kênh sự kiện mạng riêng**. Kênh đó **mất gói khi bị dồn (burst)**.
- Burst lớn nhất = "một `SendInput` cho cả hợp âm" → mọi note-on rơi vào **đúng một game-tick** → kênh
  mạng coalesce/rate-limit → rớt bớt cho người nghe. Hợp âm càng nhiều phím, burst càng nặng → khớp
  chính xác triệu chứng.

→ Trong giới hạn **SendInput-only** (P0), đòn bẩy hợp lệ duy nhất là **giãn vi mô các note-on trong
cùng một hợp âm** để mỗi phím có tick/gói mạng riêng. Đây là **đánh đổi** (đồng thời local ↔ độ tin
cậy remote), nên mặc định **TẮT**.

> Lưu ý đây **không** phải làm tròn frame (đã retired, vô nghĩa — `docs/timing-principles.md` §6) và
> **không** đổi hold. Nó chỉ tách thời điểm các note-on của *cùng một hợp âm*; onset của phím đầu giữ
> nguyên, không bao giờ dời sớm.

## 2. Cơ chế triển khai

- Knob ở **policy** (vì đây là biến đổi *timeline*, thuộc scheduler thuần):
  - `chord_stagger_us` — bước giãn mỗi phím (µs). `0` = TẮT (mặc định, một `SendInput`/hợp âm).
  - `chord_stagger_max_us` — trần tổng độ trải (mặc định `15000` ≈ 15ms, **dưới ngưỡng cảm nhận
    đồng thời ~20–30ms** → vẫn nghe ra "hợp âm", không thành rải).
- Biến đổi nằm ở `scheduler.apply_chord_stagger` (Stage 6 của `build_key_actions`), chạy **sau** khi
  tính metric (để `max_polyphony` phản ánh kích thước hợp âm logic, không phải các down đơn sau khi
  tách). Phím thứ `i` của hợp âm dời `min(i·step, max)`; phím vượt trần dồn chung tick cuối (vẫn ít
  hơn burst gốc). **Release không bị tách** (note-off lệch là vô hại; floor release per-key trong
  coordinator tự bám theo từng down đã giãn).
- Không gộp chéo giữa các hợp âm: mỗi hợp âm xử lý độc lập → không bao giờ trộn nốt của event lân cận.
- Toàn bộ máy lead/completion-anchor/min-hold/same-key conflict giữ nguyên vì chúng chạy trên timeline
  kết quả; mỗi down đã giãn thành batch 1 phím nên adaptive-lead áp đúng (lead 1-phím cho send 1-phím).

## 3. Cách bật

**CLI (A/B nhanh, khuyến nghị khi tinh chỉnh với người nghe thật):**
```
uv run play --chord-stagger-us 2500
uv run play --chord-stagger-us 2500 --chord-stagger-max-us 12000
```
`--chord-stagger-us 0` hoặc bỏ cờ = TẮT (hành vi hiện tại).

**Bền vững qua `config.json`** (thêm override vào một profile, ví dụ `audience_safe`):
```json
{
  "timing_profiles": {
    "audience_safe": { "chord_stagger_us": 2500, "chord_stagger_max_us": 15000 }
  }
}
```

Giá trị gợi ý ban đầu cho chơi online: `chord_stagger_us` = **2000–3000µs**. Hợp âm 6 phím ở 2500µs →
trải ~12.5ms (dưới trần 15ms). Tăng dần nếu vẫn rớt; giảm nếu nghe thành rải.

## 4. Kiểm chứng (BẮT BUỘC — harness local KHÔNG thấy hiệu ứng này)

Harness local (`scripts/measure_hold_rhythm.py`) chỉ đo lateness **local** → **không** quan sát được
rớt nốt remote. Phải đo bằng client/người nghe thứ hai:

1. Một máy/người nghe thứ hai ở cùng khu trong game (hoặc dùng `tests/audio_loopback.py` +
   `tests/analyze_onsets.py` thu audio phía người nghe).
2. Phát cùng một đoạn nhiều hợp âm dày, quét `chord_stagger_us ∈ {0, 1500, 2500, 3500}`.
3. Đếm **tỉ lệ note-on nghe được / note-on đã gửi** ở phía remote cho mỗi mức.
4. Chọn mức nhỏ nhất đạt tỉ lệ chấp nhận được mà chưa nghe ra arpeggio. Đính bảng số liệu vào đây.

**Tiêu chí thành công:** ở mức stagger đã chọn, remote drop-rate giảm rõ so với `0`, và người nghe vẫn
cảm nhận là hợp âm (không phải nốt rải). Local lateness gần như không đổi (chỉ các nốt *bên trong* hợp
âm trễ thêm ≤ trần, bound, sub-perceptual).

## 5. Kết quả đo (điền sau khi chạy §4)

| chord_stagger_us | remote heard/sent | ghi chú cảm nhận |
| ---: | ---: | --- |
| 0 | _(baseline)_ | |
| 1500 | | |
| 2500 | | |
| 3500 | | |

## 5b. CẬP NHẬT 2026-06-27 — Spreading là sai trục; nghi can mới: partial-send

Kết quả test thực tế của người dùng đã **bác bỏ hướng spreading**:

- Quét `--chord-stagger-us` từ 2500 xuống **100µs** (hợp âm 6 phím = 0.5ms tổng) → **mọi mức** đều
  làm remote **mất tính cùng lúc** dù 0.5ms dưới ngưỡng cảm nhận hàng chục lần. → **Remote re-quantize
  MỌI khe > 0** về cadence của nó; cái quyết định là "có tách sự kiện hay không", không phải độ lớn
  khe. ⇒ Simultaneity **bắt buộc** một `SendInput` nguyên khối (chính là mặc định). Knob giữ TẮT.
- Đổi hold không tác động ⇒ loại release timing và same-key suppression.
- Triệu chứng tinh: nhịp gốc rất tốt, lỗi **chỉ ở hợp âm** — "ngắt nhịp / một nốt lệch hoặc không
  được ấn". Rời rạc, chỉ chord.

→ Nghi can còn lại có thể sửa từ phía sender: **SendInput partial-send**
(`inputs.py::_send_scan_code_batch_impl` trả `sent < n`) — chỗ DUY NHẤT tính nguyên khối hợp âm bị
phá; phần còn lại phải đi lần `SendInput` thứ hai → hợp âm tách → remote desync. Hiếm + chỉ chord =
khớp.

### Đã thêm instrument (an toàn, chỉ quan sát)
Bộ đếm trong `inputs.py`: `partial_send_events`, `chord_split_events` (n>1 và 0<sent<n — đúng "chord
split"), `keys_deferred`, `zero_progress_retries`. Reset đầu mỗi lần phát; log cuối mỗi lần phát.

### Cách chạy thí nghiệm quyết định
```
uv run play --debug-playback        # KHÔNG dùng --chord-stagger-us (để 0)
```
Phát một bài nhiều hợp âm, người nghe đánh dấu lúc nghe lỗi. Sau đó mở `logs/playback_debug_*.log`:
- Tìm dòng `[input] CHORD SPLIT: ...` và dòng tổng `[input] SEND DIAGNOSTICS (this run): ...`.
- **Nếu `chord_split_events > 0` và trùng thời điểm nghe lỗi** ⇒ nguyên nhân **sender-side, sửa được**:
  retry phần còn lại tức thì (rút khe tách), và loại điều kiện gây partial (cùng mức UIPI với Sky,
  giữ foreground, không bị process integrity cao chặn input).
- **Nếu `chord_split_events == 0`** ⇒ lỗi **phía remote** (cấp phát voice khi nhận một gói nhiều
  note-on), sender không chạm tới được; bản gốc đã ở tối ưu SendInput-only — dừng đúng chỗ.

## 6. Ngoài phạm vi / lưu ý

- Đây mở lại có kiểm soát điều mà `docs/chord-dispatch-refactor-plan-2026-06-26.md` §0.1 khóa ("một
  hợp âm = một SendInput") — nhưng khóa đó thuộc *một task khác* (refactor hold/timbre, mục tiêu không
  đụng grouping). Task này được người dùng giao **đúng việc xét lại grouping cho bài toán remote-drop**,
  và vẫn tôn trọng P0 (SendInput-only) + không làm tròn frame.
- Suy luận cơ chế mạng (§1) dựa trên hành vi quan sát + tri thức cộng đồng Sky, **không** đọc được
  netcode đóng của game. Vì vậy mặc định TẮT và yêu cầu đo (§4) trước khi bật cho người dùng.

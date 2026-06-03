# Timing Experiments — Proof and Open Calibration

Tài liệu này là hướng dẫn thực hành để **chứng minh** các giá trị timing đang được coi là chân
lý (Part 1) và để **đo chính xác** những con số còn đang đặt bằng tai hoặc ngoại suy — chủ yếu là
**sàn hold/gap remote thật** và đặc tả jitter mạng (Part 2).

Công cụ: **Audacity** để thu và tách onset; một **máy thứ hai** đóng vai người nghe trong phòng
online. Mọi bước đều viết để bạn làm theo trực tiếp.

> Nguyên tắc tối cao: sự thật nằm ở **âm thanh game thu được**, không phải log. Nếu một kết quả đo
> ở đây mâu thuẫn với một "quy tắc kinh nghiệm" trong `timing-principles.md`, **kết quả đo thắng**.

> ⚠️ **CẬP NHẬT KIẾN TRÚC (2026-06) — đọc trước khi dùng file này.** Sau đợt audit + refactor 3 phase
> (xem [`timing-architecture-audit.md`](timing-architecture-audit.md)), **ba cần gạt đã bị XOÁ khỏi
> code**: `input_lead` (no-op kiến trúc — player tự sinh timeline, không có mốc tham chiếu ngoài),
> `chord_merge` (gần như không fire trên bài thật), `frame_align` (off mọi profile + vô nghĩa). Các
> thí nghiệm gắn với chúng (**O1, O2, O5**, và phần lớn **O4**) giờ **OBSOLETE** — giữ lại làm hồ sơ,
> không chạy nữa. Mọi flag `--input-lead-ms` / `--chord-merge-window-ms` / `--frame-align` đã biến
> mất. Mô hình local còn đúng **3 cần gạt thật**: `min_hold` (sàn visibility), `repeat_release_gap`
> (sàn same-key), `release_gap`. Phần CÒN MỞ thực sự: **O3, O6, O7, O9** (+ O8 đã re-frame).

---

## 0. Setup and tooling (làm một lần)

### 0.1 Build the test songs

Chạy một lần để sinh các bài `songs/TEST_*.json`:

```
uv run python tests/make_test_song.py
```

Các bài cần dùng (đã có sẵn trong script):

| Song                     | Hình dạng                                           | Dùng cho                              |
| ------------------------ | --------------------------------------------------- | ------------------------------------- |
| `TEST_visibility`        | 15 nốt đơn, cách 700 ms                             | sàn hold/visibility (T1)              |
| `TEST_repeat_gap`        | 1 phím, các block gap giảm dần (50→5 ms)            | sàn same-key gap (T2, O6)             |
| `TEST_repeat_staircase`  | 1 phím, interval giảm dần                           | drop same-key A/B (T2, T4)            |
| `TEST_metro_alt_200/120` | nhịp đều, **đan xen 2 phím** (không dính gap floor) | jitter onset / cảm giác nhịp (T3, O4) |
| `TEST_metro_same_200`    | nhịp đều, 1 phím                                    | survivability same-key remote (O3)    |
| `TEST_chords`            | hợp âm 2..6 phím                                    | chord remote (O8)                     |

Hai bài bổ sung (đã có sẵn trong script):

| Song                   | Hình dạng                             | Dùng cho                                            |
| ---------------------- | ------------------------------------- | --------------------------------------------------- |
| `TEST_metro_alt_500`   | nhịp 120 BPM (500 ms), đan xen 2 phím | ~~O1/O5 (lead)~~ — giờ chỉ còn hữu ích cho O7 jitter |
| `TEST_rolled_chord_18` | hợp âm "rải" 4 phím cách 18 ms        | ~~O2 (chord_merge)~~ **OBSOLETE** — chord_merge đã xoá |

> Hai bài này sinh ra cho các thí nghiệm đã OBSOLETE. Giữ trong script (vô hại); `TEST_metro_alt_500`
> còn dùng được cho O7. `TEST_rolled_chord_18` giờ chỉ minh hoạ: nốt cách 18 ms nay **luôn đi riêng**
> (không còn bị gom) — đúng behavior mới.

### 0.2 Recording the game audio (Audacity, Windows WASAPI loopback)

Cách sạch nhất để thu đúng tiếng game trên Windows, không cần "Stereo Mix":

1. Mở Audacity. Trên thanh **Audio Setup** (hoặc Device Toolbar) chọn **Host = Windows WASAPI**.
2. Chọn **Recording Device = "<thiết bị loa của bạn> (loopback)"** (ví dụ `Speakers (loopback)`).
   Loopback thu đúng những gì phát ra loa đó — tức tiếng game.
3. Đặt **Project Rate = 48000 Hz**, kênh **Mono** là đủ (onset không cần stereo).
4. Trong game Sky, chọn **nhạc cụ gõ/tắt nhanh** (percussive, decay ngắn) — KHÔNG dùng nhạc cụ
   ngân dài, vì đuôi ngân chồng lên nhau sẽ không tách được onset (xem 0.6).
5. Bấm **Record (R)** trước, đợi ~1 giây, rồi mới chạy lệnh player. Bấm **Stop** sau khi xong.

> Mẹo: để mỗi lần thu gọn, dùng `--countdown 3` và để 1–2 giây im lặng đầu/cuối; im lặng giúp
> Label Sounds không dính nhiễu.

### 0.3 Extracting onsets (Label Sounds → Export Labels)

1. Chọn toàn bộ track (Ctrl+A).
2. **Analyze ▸ Label Sounds** (bản cũ: _Sound Finder_). Tham số gợi ý ban đầu:
   - _Threshold (dB)_: khoảng **-30 dB** (hạ xuống -40 nếu thiếu onset, nâng lên -25 nếu bắt nhầm
     nhiễu).
   - _Minimum silence between sounds_: đặt **nhỏ hơn khoảng cách nốt** (ví dụ 0.05 s cho bài 200 ms).
   - _Label type_: "Label before sound" / start of sound.
3. Kiểm mắt: số nhãn phải khớp số nốt mong đợi. Nếu lệch, chỉnh threshold/min-silence và chạy lại
   (xem **O9** để hiệu chuẩn detector trước khi tin số liệu).
4. **File ▸ Export Other ▸ Export Labels…** → lưu `labels.txt` (mỗi dòng: `start <tab> end <tab>
nhãn`). `start` chính là thời điểm onset.

### 0.4 The analysis script

`tests/analyze_onsets.py` tính IOI (khoảng cách giữa các onset) và, nếu đưa thêm file telemetry
CSV, tính cả **game-only jitter** (IOI thu được trừ IOI phía gửi → khử nhiễu của player):

```
# chỉ phân tích onset thu được:
uv run python tests/analyze_onsets.py labels.txt

# khử nhiễu phía gửi (cần chạy player với --debug-csv, file ở logs/):
uv run python tests/analyze_onsets.py labels.txt logs/playback_telemetry_XXXX.csv
```

Đọc kết quả:

- `[GAME] IOI mean/std/spread` — độ đều của nhịp thu được (std nhỏ = đều).
- `[SENT] IOI std` — độ đều phía player gửi (phải rất nhỏ, ~<0.1 ms).
- `[GAME-only jitter] std` + `residuals` — phần dao động THUẦN do game. Nếu residuals có **dao
  động chu kỳ** (lặp +X/−X) → đó là beating của tick nội bộ; nếu ngẫu nhiên → nhiễu thường.

### 0.5 Two-computer remote setup (host + listener)

- **Máy A (host)**: chạy game Sky + player tool, vào một phòng online, phát nhạc bằng player.
- **Máy B (listener)**: máy khác, **cùng phòng online**, nghe tiếng đàn của A replicate sang, và
  **thu chính tiếng game của B** bằng Audacity (mục 0.2). Đây là "sự thật phía remote".
- Hai máy có **đồng hồ riêng**, không chung mốc thời gian. Vì vậy:
  - Đo **độ đều / mất nốt** dùng được trực tiếp (IOI là tương đối, đếm onset là tuyệt đối).
  - Đo **độ trễ tuyệt đối** (O1 remote) phải dùng metronome tham chiếu ở phía B (mục 0.7), không
    so trực tiếp đồng hồ A với B.
- Cố định điều kiện mạng giữa các lần A/B (cùng Wi-Fi/ethernet, cùng số người trong phòng) để so
  sánh công bằng; ghi lại ping nếu đo được.

### 0.6 Universal controls (checklist mỗi lần đo)

1. **Khóa FPS game từ bên ngoài** (VSync hoặc RTSS/frame limiter), xác nhận bằng overlay đếm FPS.
2. Khi đo **bản chất của game** (T1/T2/T3): **KHÔNG** truyền `--fps` cho player (giữ giá trị thô —
   nếu truyền, frame-aware sẽ rescale biến đang quét và che kết quả). Khi đo **logic FPS-aware của
   player**: **CÓ** truyền `--fps`.
3. Khi quét gap (T2): giữ hold **≥ 1 frame ở FPS test** để lỗi visibility không lẫn vào biến gap.
4. Dùng nhạc cụ **gõ/tắt nhanh**; đếm **onset**, không đếm độ to.
5. Mỗi lần thu chạy kèm `--debug-csv` để có số liệu phía gửi đối chiếu.

### 0.7 Metronome reference (đo độ trễ tuyệt đối)

> Trước đây mục này phục vụ O1 (calibrate input_lead). **input_lead đã bị xoá** nên phần calibrate
> lead không còn ý nghĩa (player không có mốc tham chiếu ngoài để "đến sớm" — xem audit doc §1).
> Phương pháp metronome vẫn giữ vì còn cần để đo **offset/độ trễ tuyệt đối** ở O6/O7 và để kiểm tra
> phía remote nếu sau này quay lại O3/O4. **Bỏ qua mọi nhắc tới `--input-lead-ms` bên dưới.**

Để biết nốt vào **sớm/trễ so với phách** bao nhiêu, cần một mốc nhịp nằm cùng dòng thời gian với
tiếng game. Hai cách, từ chính xác đến đơn giản:

**Cách A — Audacity overdub (khách quan):**

1. Trong Audacity: **Generate ▸ Rhythm Track**, đặt **Tempo = BPM của bài** (bài
   `TEST_metro_alt_500` = 500 ms/nốt = **120 BPM**), tạo một click track.
2. **Hiệu chuẩn latency một lần**: Edit ▸ Preferences ▸ Devices ▸ Latency, làm bài test latency của
   Audacity (thu lại chính click qua loopback, đo lệch, nhập vào _Latency correction_). Bước này để
   track thu mới được canh đúng vào dòng thời gian của click.
3. **Tách đường tiếng**: cho **game phát ra loa** (Audacity loopback thu loa này), còn **click của
   Audacity phát ra tai nghe** (thiết bị khác) để click KHÔNG lọt vào bản thu. Bật **Transport ▸
   Transport Options ▸ Overdub**.
4. Record: Audacity vừa phát click (bạn nghe ở tai nghe làm mốc) vừa thu tiếng game vào track mới,
   đã canh thời gian theo click.
5. Tách onset game (0.3). Với mỗi nốt, **offset = onset_game − click gần nhất**. Offset trung bình
   < 0 = nốt vào sớm, > 0 = vào trễ.

**Cách B — bằng tai (đơn giản):** ~~bật metronome 120 BPM, phát `TEST_metro_alt_500`, chỉnh
`--input-lead-ms` đến khi nghe nốt trùng phách.~~ **OBSOLETE** — không còn flag lead để chỉnh. Giờ
nốt luôn phát đúng tại `source_time` (không dịch); nếu nghe lệch phách thì đó là tick ~60 Hz của
game (T3) hoặc trễ mạng, **không sửa được từ player**.

---

## Part 1 — Confirmed truths (chạy lại để kiểm chứng)

Mỗi chân lý dưới đây đã được mã hóa trong `config.py`/`FrameTimingDefaults` và ghi ở
`timing-principles.md` Appendix A. Đây là cách tái lập bằng chứng.

### T1 — Game samples input once per render frame; visibility floor = 1 frame

- **Mục tiêu:** chứng minh nốt phải DOWN đủ lâu để bị lấy mẫu ở biên frame.
- **Chuẩn bị:** bài `TEST_visibility`; khóa FPS ngoài ở 30/60/144; **không `--fps`**.
- **Các bước:** với mỗi FPS, quét hold giảm dần — chạy nhiều lần:
  ```
  uv run python -m main --song TEST_visibility --hold-ms 40 --min-hold-ms 40 --debug-csv
  uv run python -m main --song TEST_visibility --hold-ms 20 --min-hold-ms 20 --debug-csv
  uv run python -m main --song TEST_visibility --hold-ms 10 --min-hold-ms 10 --debug-csv
  ...
  ```
  (Phải đặt CẢ `--hold-ms` lẫn `--min-hold-ms` vì clamp hold hợp nhất lấy `max(min_hold, …)`.)
  Thu mỗi lần, đếm onset.
- **Đo:** hold nhỏ nhất mà vẫn đủ 15/15 onset.
- **✅ KẾT QUẢ ĐÃ ĐO (result.md):** @30 reliable 32 ms (rớt 31), @60 reliable 16 ms (rớt 15), @144
  reliable 7 ms (rớt 6) → sàn thật ≈ **0.96–1.01 frame** ở cả ba FPS (tuyến tính sạch). Hold dưới mép
  đăng ký theo xác suất.
- **Kết luận:** game đọc **state** theo frame, sàn visibility = **1.0 frame**. Code dùng
  `min_visible_hold_frames = 1.25` (= 1 frame + ~25% biên). **ĐÃ ÁP DỤNG (Phase 3):** riêng
  `local_precise` hạ ratio xuống **1.1** (sắc hơn ~2.5 ms @60, vẫn trên sàn đo 16 ms); 3 profile khác
  giữ 1.25.

### T2 — Same-key repeat gap floor = max(~1.5 frame, ~18 ms)

- **Chuẩn bị:** `TEST_repeat_gap` (hold cố định 24 ms, gap giảm 50→5 ms) hoặc `TEST_repeat_staircase`;
  khóa FPS ngoài; hold giữ ≥ 1 frame ở FPS test.
- **Các bước:** `uv run python -m main --song TEST_repeat_gap --debug-csv` ở 60 rồi 144 (không `--fps`).
  Thu, đếm onset MỖI block (mỗi block 10 lần bấm cùng phím).
- **Đo:** block có gap nhỏ nhất mà vẫn đủ 10/10 onset.
- **✅ KẾT QUẢ ĐÃ ĐO (result.md):** @60 reliable **24 ms** (1.44 frame), @144 reliable **16 ms** (2.3
  frame). Ở 144 gap tin cậy >> 1.5 frame → bị **tường thời gian cố định ~16–18 ms** chi phối (= chu kỳ
  tick ~60 Hz, nối với T3), không phải bội số frame.
- **Kết luận:** `gap ≥ max(1.5×frame, 18000 µs)` → khớp đo @60 (model 25001 ≈ đo 24), hơi bảo thủ @144
  (model 18000 vs đo 16). Giữ `repeat_release_gap_frames = 1.5`, `repeat_release_gap_floor_us = 18000`.

### T3 — Onset cadence is a fixed ~60 Hz tick (game behavior, player không sửa được)

> Hệ quả cũ "input lead không scale theo FPS" giờ MOOT (lead đã xoá). Nhưng phát hiện gốc về **tick
> ~60 Hz của game** vẫn đứng vững và là lý do nền tảng để KHÔNG cố "đẩy nốt cho đúng nhịp" từ player.

- **EXP-1 (phía gửi sạch):** `TEST_metro_alt_120` ở 60 và 144 với `--debug-csv`; phân tích CHỈ CSV.
  ✅ Đo: send-interval std ~0.036–0.052 ms, lateness p95 ~0.07–0.11 ms → **vấn đề nhịp KHÔNG do player**.
- **EXP-2 (jitter onset không giảm theo FPS):** `TEST_metro_alt_200`, **không `--fps`**, khóa FPS
  ngoài 60 rồi 144; thu + `analyze_onsets.py labels.txt csv`.
  - ✅ **ĐÍNH CHÍNH so với kỳ vọng cũ:** jitter KHÔNG phải sàn cố định ~12–13 ms. Thực đo là **lưỡng
    cực / phụ thuộc pha**: có run sạch (residual std ~0.02 ms: 60fps-01, 144fps-02), có run dính
    **bucket-jump ~20 ms** (60fps-02 std 5.35 ms; 144fps-01 std 8.25 ms, IOI cụm 181/219 ms). Quan
    trọng: bucket-jump **không giảm khi lên 144** → không bám render frame.
- **Kết luận:** onset bị **tick nội ~60 Hz** lượng tử hoá theo pha, độc lập render FPS ≥60. Đây là
  **hành vi game, player không sửa được** — không dịch đều (lead) hay snap frame (frame_align) nào
  chữa được scatter tương đối. Chính vì vậy 3 cần gạt onset đó đã bị xoá.

### T4 — Wide audience floors were not shown necessary (remote)

- **Chuẩn bị:** phòng online, máy B thu (0.5). A/B `audience_safe` vs `local_precise` (profile
  `audience_frame_test` đã bị xoá ở đợt rút gọn profile). LƯU Ý: sau refactor, audience_safe khác
  local_precise CHỈ ở sàn hold/min_hold/repeat_gap + release_gap (không còn khác lead/chord).
- **Các bước:** host phát `TEST_metro_same_200` rồi `TEST_repeat_staircase` lần lượt 2 profile; B thu
  từng lần.
- **Đo (trên WAV của B):** đếm onset mỗi bài; đo "valley drop" giữa 2 onset same-key (độ tụt
  envelope) xem có đủ down-up-down.
- **Kết quả đã có (2 phiên):** floor thấp pass gần ngang floor cao, không mất nốt hệ thống.
- **Kết luận:** dưới mạng đã test, sàn rộng không cần. Chưa stress mạng xấu → xem **O3/O7**.

---

## Part 2 — Open calibration experiments

Phần này thay các con số đang đoán bằng số đo. **Trạng thái sau refactor:** O1/O2/O5 đã OBSOLETE
(knob bị xoá); O4 re-frame; còn mở thực sự = **O3, O6, O7, O8, O9**.

### O1 — ~~`input_lead_us`: đo độ trễ thật~~ **[OBSOLETE — knob đã xoá]**

> **Kết luận đã chốt (O1 + audit §1):** input_lead là **no-op kiến trúc**. Engine zero-base đồng hồ về
> lúc bấm play; scheduler dịch đều mọi nốt rồi clamp nốt đầu `max(0, source − lead)`. Player tự sinh
> toàn timeline, **không có mốc tham chiếu ngoài** → dịch đều là không quan sát được (chỉ để lại
> artifact nén khoảng mở đầu). Thực đo: lead 0/8/20 cho offset y như nhau (~20 ms — là DAW/tick game,
> không do lead). **Đã XOÁ input_lead khỏi code (Phase 1).** Không còn gì để calibrate ở đây.
> Bài học giữ lại: lead chỉ dịch trung bình, KHÔNG sửa được scatter tương đối (jitter T3) — đó mới là
> thứ tai nghe là "lạc nhịp".

### O2 — ~~`chord_merge_window_us`: ngưỡng làm bẹt~~ **[OBSOLETE — knob đã xoá]**

> **Kết luận đã chốt (O2):** ngưỡng làm bẹt nằm trong **10–20 ms** (0/10 ms giữ rải; 20/30 ms gom).
> Local 2500 / audience 5000 đều an toàn dưới ngưỡng — NHƯNG bài thật **không có cụm 5–20 ms** (nốt
> hoặc trùng giờ, hoặc cách ~100 ms) nên chord_merge **gần như không bao giờ fire**. **Đã XOÁ
> chord_merge (Phase 2);** chord trùng timestamp vẫn gom ở bước event-grouping cuối, nốt lệch ≥ vài ms
> đi riêng — đúng ý đồ. Không còn cần đo.

### O3 — The real remote no-drop floor (sàn hold/gap audience theo số liệu) — **CÒN MỞ**

> **Trạng thái (result.md):** user **gác có chủ đích** để làm chuẩn local trước ("xử lý tốt local thì
> remote cũng tốt"). Sau refactor, audience_safe khác local_precise chỉ ở sàn → đây là nơi DUY NHẤT
> còn lý do tồn tại của audience_safe; cần O3 để chứng minh sàn nào thật sự cần.

- **Vì sao mở:** sàn audience (hold 20000 / repeat 24000) là ngoại suy; `local_precise` (visibility
  1.1 frame / repeat 18000) **đôi khi mất nốt** remote. Ranh giới chưa đo.
- **Chuẩn bị:** máy B thu; `TEST_metro_same_200` + `TEST_repeat_staircase`.
- **Các bước:** giữ các tham số khác = local, nâng RIÊNG từng cái:
  - hold: `--hold-ms 0 → 8 → 12 → 16 → 20` (kèm `--min-hold-ms` tương ứng)
  - repeat_gap: `--repeat-release-gap-ms 18 → 20 → 22 → 24`
    Mỗi mức chạy N lần (gồm cả một lần mạng đông/kém). B đếm onset mất.
- **Quyết định:** hold/repeat_gap **nhỏ nhất mà 0 drop qua mọi lần** = sàn audience thật, thay ngoại
  suy ở T4/A.9. Chỉ nâng cái nào chặn drop; đừng nâng cái khác.

### O4 — The local-vs-audience on-beat puzzle — **PHẦN LỚN ĐÃ GIẢI (bằng suy luận sau refactor)**

- **Hiện tượng (báo cáo thực tế):** qua `local_precise`, máy người nghe thấy **chặt và đúng nhịp
  hơn** `audience_safe`, dù local **đôi khi mất nốt**.
- **Giả thuyết cũ đã BỊ BÁC một phần:** nghi phạm chính từng là **chord_merge** + **input_lead**.
  Cả hai giờ đã chứng minh vô hại/no-op và **đã xoá**. → Sau refactor, onset của local và audience
  **giống hệt nhau** (cùng = `source_time`, không còn lead/chord làm khác). Vậy audience **không thể**
  "lạc nhịp tương đối" so với local từ phía player.
- **Cái còn lại có thể gây "lỏng":**
  1. **hold dài hơn** của audience (20000 vs 1.1f) → key chồng lấn nhiều hơn = cảm giác "mushy", dù
     KHÔNG dịch onset;
  2. **trễ + jitter mạng** + bucket-jump ~20 ms của game (T3) — run-to-run, không phải local-vs-audience;
- **Còn cần đo (nếu quay lại nhánh remote):** A/B một-biến giữa local và audience, giờ chỉ còn **2 cần
  gạt** để thử (làm mù, máy B chấm "đúng nhịp" 1–5 + đếm drop):
  - V1: local + `--hold-ms 20 --min-hold-ms 18`  (kiểm "mushy" do hold dài)
  - V2: local + `--repeat-release-gap-ms 24`      (kiểm an toàn same-key)
- **Quyết định:** nếu V1 làm điểm nhịp tệ hơn mà không giảm drop → hold dài là thủ phạm "lỏng", giữ
  audience hold sát local. Tổng hợp với **O3**: audience đúng ≈ local + mức nâng tối thiểu chặn drop.
- **Ghi chú làm mù:** host phát cùng bài dưới 2 cấu hình thứ tự ngẫu nhiên, tên file trung tính
  (`A1.wav`, `A2.wav`); người chấm nghe xong mới tra cấu hình. Lặp ≥3 vòng.

### O5 — ~~The `<60 FPS` input-lead assumption~~ **[OBSOLETE — knob đã xoá]**

> input_lead bị xoá; nhánh `<60` nâng lead về `½ frame` (`input_lead_min_frame_ratio`) cũng đã gỡ ở
> Phase 1. Không còn lead để đo ở FPS thấp. (Sàn hold/gap ở 30 FPS vẫn cần đo — xem **O6**.)

### O6 — Clean 30 FPS same-key gap point (mục A.8 còn treo)

- **Vì sao mở:** T2 mới có 60 và 144; model dự đoán gap ~46 ms @30 (hold phải ≥ 42 ms) nhưng chưa đo.
- **Các bước:** `TEST_repeat_gap` ở khóa 30 FPS, đặt `--hold-ms 45 --min-hold-ms 45` (≥ 1 frame@30),
  đếm onset mỗi block.
- **Quyết định:** ghi gap 100%-tin-cậy @30 vào bảng A.4.

### O7 — Network delay & jitter characterization (mới)

- **Vì sao thêm:** O3 cần biết mạng tệ đến đâu mới đặt được biên an toàn cho sàn. Cần đặc tả phân bố
  trễ/jitter của replication, không chỉ "pass/fail".
- **Các bước:** host phát `TEST_metro_alt_200` (nhịp đều). Máy A thu local (qua loopback) ĐỒNG THỜI
  máy B thu remote. Vì 2 đồng hồ khác nhau, dùng **IOI**: so std(IOI) của B với std(IOI) của A trên
  cùng chuỗi nốt.
- **Đo:** `jitter_mạng ≈ sqrt(std_B² − std_A²)` (gần đúng, coi nhiễu độc lập). Lặp ở vài điều kiện
  mạng (vắng/đông người, Wi-Fi/ethernet).
- **Quyết định:** biên cộng thêm cho sàn O3 nên ≳ jitter mạng đo được (vd 2σ). Nếu jitter mạng nhỏ
  so với bucket-jump ~20 ms phụ-thuộc-pha của game (T3) → sàn rộng càng vô ích.

### O8 — Remote chord integrity — **RE-FRAME (không còn chord_merge để so)**

- **Vì sao giữ:** mục tiêu lớn của audience là hợp âm nghe đủ/không vỡ ở máy khác — vẫn là câu hỏi
  hợp lệ, ĐỘC LẬP với chord_merge (knob đó đã xoá). Giờ chord = các nốt **cùng timestamp** được gửi
  trong một SendInput (gom ở event-grouping cuối).
- **Các bước:** host phát `TEST_chords` (hợp âm 2..6 phím) qua local vs audience; B thu.
- **Đo:** mỗi hợp âm B nghe đủ số nốt không (đếm thành phần phổ tại mỗi onset), có "rattly/vỡ" không.
  Biến duy nhất còn khác giữa local/audience ảnh hưởng chord là **hold** (dài hơn) — kiểm xem hold dài
  có giúp/làm hại độ đủ nốt remote không.
- **Quyết định:** nếu hold dài KHÔNG cải thiện độ đủ nốt remote → thêm bằng chứng để giữ audience hold
  sát local (củng cố O4).

### O9 — Onset-detector calibration (control — làm trước khi tin số liệu)

- **Vì sao thêm:** mọi kết luận dựa trên Label Sounds; phải biết detector đếm đúng.
- **Các bước:** phát `TEST_visibility` (15 nốt rõ, cách 700 ms) với hold an toàn (vd `--hold-ms 30`,
  60 FPS). Thu, chạy Label Sounds.
- **Đo:** số nhãn phải = 15, và IOI mean ≈ 700 ms, std nhỏ. Tinh chỉnh threshold/min-silence cho ra
  đúng 15 trước khi dùng detector cho các bài khó hơn.
- **Quyết định:** chốt một bộ tham số Label Sounds chuẩn cho nhạc cụ bạn dùng; ghi lại để mọi thí
  nghiệm dùng cùng cấu hình.

---

## Part 3 — Folding results back

- **Chỉ còn 3 cần gạt để chỉnh trong `config.py`:** `min_hold` (sàn visibility, qua `*_frames`/
  `*_floor_us`), `repeat_release_gap`, `release_gap`. `input_lead`/`chord_merge`/`frame_align` đã bị
  xoá — đừng thêm lại; nếu một thí nghiệm "cần" chúng, hãy hỏi lại giả thuyết (xem audit doc §1–2).
- Chỉ hạ một sàn **sau khi** O3 chứng minh 0 drop ở mức thấp hơn; không bao giờ hạ sàn chỉ để nhìn
  "nhanh hơn" (nguyên tắc §18/§20 của `timing-principles.md`).
- Khi một số đo đã chốt: cập nhật `config.py` (và `FrameTimingDefaults` nếu là tỷ lệ toàn cục) cùng
  dòng tương ứng trong `timing-principles.md` Appendix A, trích mã thí nghiệm (O3/O6/O7) ở đây.
- Tick ~60 Hz và bộ lấy mẫu theo frame là **hành vi của game** — chạy lại các thí nghiệm Part 1 như
  regression sau mỗi lần game cập nhật.

---

## Appendix — Results template (copy mỗi lần đo)

```
Thí nghiệm: __  | Ngày: __  | Máy thu: A(local)/B(remote)
Game FPS (khóa ngoài): __   | Nhạc cụ: __   | Label Sounds threshold/min-silence: __
Profile + flags: __
Bài: __   | Số nốt mong đợi: __   | Onset thu được: __
analyze_onsets: GAME std=__  SENT std=__  GAME-only jitter std=__  residuals chu kỳ? __
Kết luận / quyết định: __
```

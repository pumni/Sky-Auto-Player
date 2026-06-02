# Timing Experiments — Proof and Open Calibration

Tài liệu này là hướng dẫn thực hành để **chứng minh** các giá trị timing đang được coi là chân
lý (Part 1) và để **đo chính xác** những con số còn đang đặt bằng tai hoặc ngoại suy — đặc biệt
`input_lead_us`, `chord_merge_window_us`, và sàn hold/gap remote thật (Part 2).

Công cụ: **Audacity** để thu và tách onset; một **máy thứ hai** đóng vai người nghe trong phòng
online. Mọi bước đều viết để bạn làm theo trực tiếp.

> Nguyên tắc tối cao: sự thật nằm ở **âm thanh game thu được**, không phải log. Nếu một kết quả đo
> ở đây mâu thuẫn với một "quy tắc kinh nghiệm" trong `timing-principles.md`, **kết quả đo thắng**.

---

## 0. Setup and tooling (làm một lần)

### 0.1 Build the test songs

Chạy một lần để sinh các bài `songs/TEST_*.json`:

```
python tests/make_test_song.py
```

Các bài cần dùng (đã có sẵn trong script):

| Song | Hình dạng | Dùng cho |
| --- | --- | --- |
| `TEST_visibility` | 15 nốt đơn, cách 700 ms | sàn hold/visibility (T1) |
| `TEST_repeat_gap` | 1 phím, các block gap giảm dần (50→5 ms) | sàn same-key gap (T2, O6) |
| `TEST_repeat_staircase` | 1 phím, interval giảm dần | drop same-key A/B (T2, T4) |
| `TEST_metro_alt_200/120` | nhịp đều, **đan xen 2 phím** (không dính gap floor) | jitter onset / cảm giác nhịp (T3, O4) |
| `TEST_metro_same_200` | nhịp đều, 1 phím | survivability same-key remote (O3) |
| `TEST_chords` | hợp âm 2..6 phím | chord remote (O8) |

Hai bài bổ sung cho Part 2 (đã có sẵn trong script):

| Song | Hình dạng | Dùng cho |
| --- | --- | --- |
| `TEST_metro_alt_500` | nhịp 120 BPM (500 ms), đan xen 2 phím | đo độ trễ tuyệt đối bằng metronome (O1, O5) |
| `TEST_rolled_chord_18` | hợp âm "rải" 4 phím cách 18 ms | ngưỡng gom chord (O2, O8) |

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
2. **Analyze ▸ Label Sounds** (bản cũ: *Sound Finder*). Tham số gợi ý ban đầu:
   - *Threshold (dB)*: khoảng **-30 dB** (hạ xuống -40 nếu thiếu onset, nâng lên -25 nếu bắt nhầm
     nhiễu). 
   - *Minimum silence between sounds*: đặt **nhỏ hơn khoảng cách nốt** (ví dụ 0.05 s cho bài 200 ms).
   - *Label type*: "Label before sound" / start of sound.
3. Kiểm mắt: số nhãn phải khớp số nốt mong đợi. Nếu lệch, chỉnh threshold/min-silence và chạy lại
   (xem **O9** để hiệu chuẩn detector trước khi tin số liệu).
4. **File ▸ Export Other ▸ Export Labels…** → lưu `labels.txt` (mỗi dòng: `start <tab> end <tab>
   nhãn`). `start` chính là thời điểm onset.

### 0.4 The analysis script

`tests/analyze_onsets.py` tính IOI (khoảng cách giữa các onset) và, nếu đưa thêm file telemetry
CSV, tính cả **game-only jitter** (IOI thu được trừ IOI phía gửi → khử nhiễu của player):

```
# chỉ phân tích onset thu được:
python tests/analyze_onsets.py labels.txt

# khử nhiễu phía gửi (cần chạy player với --debug-csv, file ở logs/):
python tests/analyze_onsets.py labels.txt logs/playback_telemetry_XXXX.csv
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

### 0.7 Metronome reference (cần cho O1 — đo độ trễ tuyệt đối)

Để biết nốt vào **sớm/trễ so với phách** bao nhiêu, cần một mốc nhịp nằm cùng dòng thời gian với
tiếng game. Hai cách, từ chính xác đến đơn giản:

**Cách A — Audacity overdub (khách quan):**
1. Trong Audacity: **Generate ▸ Rhythm Track**, đặt **Tempo = BPM của bài** (bài
   `TEST_metro_alt_500` = 500 ms/nốt = **120 BPM**), tạo một click track.
2. **Hiệu chuẩn latency một lần**: Edit ▸ Preferences ▸ Devices ▸ Latency, làm bài test latency của
   Audacity (thu lại chính click qua loopback, đo lệch, nhập vào *Latency correction*). Bước này để
   track thu mới được canh đúng vào dòng thời gian của click.
3. **Tách đường tiếng**: cho **game phát ra loa** (Audacity loopback thu loa này), còn **click của
   Audacity phát ra tai nghe** (thiết bị khác) để click KHÔNG lọt vào bản thu. Bật **Transport ▸
   Transport Options ▸ Overdub**.
4. Record: Audacity vừa phát click (bạn nghe ở tai nghe làm mốc) vừa thu tiếng game vào track mới,
   đã canh thời gian theo click.
5. Tách onset game (0.3). Với mỗi nốt, **offset = onset_game − click gần nhất**. Offset trung bình
   < 0 = nốt vào sớm, > 0 = vào trễ.

**Cách B — bằng tai (đơn giản, đúng cách 14500 ban đầu được chọn):** bật metronome 120 BPM, phát
`TEST_metro_alt_500`, chỉnh `--input-lead-ms` đến khi nghe nốt **trùng phách**. Đây là cách nhanh,
chấp nhận được vì lead vốn là đại lượng cảm nhận.

---

## Part 1 — Confirmed truths (chạy lại để kiểm chứng)

Mỗi chân lý dưới đây đã được mã hóa trong `config.py`/`FrameTimingDefaults` và ghi ở
`timing-principles.md` Appendix A. Đây là cách tái lập bằng chứng.

### T1 — Game samples input once per render frame; visibility floor = 1 frame

- **Mục tiêu:** chứng minh nốt phải DOWN đủ lâu để bị lấy mẫu ở biên frame.
- **Chuẩn bị:** bài `TEST_visibility`; khóa FPS ngoài ở 30/60/144; **không `--fps`**.
- **Các bước:** với mỗi FPS, quét hold giảm dần — chạy nhiều lần:
  ```
  python -m main --song TEST_visibility --hold-ms 40 --min-hold-ms 40 --debug-csv
  python -m main --song TEST_visibility --hold-ms 20 --min-hold-ms 20 --debug-csv
  python -m main --song TEST_visibility --hold-ms 10 --min-hold-ms 10 --debug-csv
  ...
  ```
  (Phải đặt CẢ `--hold-ms` lẫn `--min-hold-ms` vì clamp hold hợp nhất lấy `max(min_hold, …)`.)
  Thu mỗi lần, đếm onset.
- **Đo:** hold nhỏ nhất mà vẫn đủ 15/15 onset.
- **Kết quả mong đợi:** ≈33 ms @30, ≈17 ms @60, ≈7 ms @144 (≈ 1 frame). Hold dưới 1 frame đăng ký
  **theo xác suất** (vd ~0.72 frame ≈ 72%).
- **Kết luận:** game đọc **state** theo frame → `min_visible_hold_frames = 1.25`, không có thành
  phần ms cố định.

### T2 — Same-key repeat gap floor = max(~1.5 frame, ~18 ms)

- **Chuẩn bị:** `TEST_repeat_gap` (hold cố định 24 ms, gap giảm 50→5 ms) hoặc `TEST_repeat_staircase`;
  khóa FPS ngoài; hold giữ ≥ 1 frame ở FPS test.
- **Các bước:** `python -m main --song TEST_repeat_gap --debug-csv` ở 60 rồi 144 (không `--fps`).
  Thu, đếm onset MỖI block (mỗi block 10 lần bấm cùng phím).
- **Đo:** block có gap nhỏ nhất mà vẫn đủ 10/10 onset.
- **Kết quả mong đợi:** gap 100%-tin-cậy ≈22–24 ms @60, ≈14–17 ms @144; ở đúng 1 frame chỉ ~80%.
- **Kết luận:** `gap ≥ max(~1.4×frame, ~17 ms)` → encode `repeat_release_gap_frames = 1.5`,
  `repeat_release_gap_floor_us = 18000`.

### T3 — Onset cadence is a fixed ~60 Hz tick (input lead must not scale with render FPS ≥60)

- **EXP-1 (phía gửi sạch):** `TEST_metro_alt_120` ở 60 và 144 với `--debug-csv`; phân tích CHỈ CSV.
  Kỳ vọng send-interval std ~0.05–0.07 ms, lateness p95 ~0.13 ms → vấn đề nhịp KHÔNG do player.
- **EXP-2 (jitter onset không giảm theo FPS):** `TEST_metro_alt_200`, **không `--fps`**, khóa FPS
  ngoài 60 rồi 144; thu + `analyze_onsets.py labels.txt csv`.
  - Kỳ vọng `[GAME-only jitter] std` ≈13 ms @60 ≈12 ms @144 (gần như không đổi — nếu bám render
    frame phải giảm ~một nửa). Residuals @144 có nhảy chu kỳ ±~20 ms.
- **Kết luận:** onset bị tick nội cố định ~60 Hz chi phối, độc lập render FPS ≥60 → lead giữ cố định
  ở FPS ≥60 (đã gỡ phase-comp; đã bỏ `high_fps_precise`). Jitter ~12 ms là của game, **không sửa
  được từ player**.

### T4 — Wide audience floors were not shown necessary (remote)

- **Chuẩn bị:** phòng online, máy B thu (0.5). A/B `audience_safe` vs `audience_frame_test` (nếu
  còn) hoặc so audience hiện tại với một profile floor thấp.
- **Các bước:** host phát `TEST_metro_same_200` rồi `TEST_repeat_staircase` lần lượt 2 profile; B thu
  từng lần.
- **Đo (trên WAV của B):** đếm onset mỗi bài; đo "valley drop" giữa 2 onset same-key (độ tụt
  envelope) xem có đủ down-up-down.
- **Kết quả đã có (2 phiên):** floor thấp pass gần ngang floor cao, không mất nốt hệ thống.
- **Kết luận:** dưới mạng đã test, sàn rộng không cần. Chưa stress mạng xấu → xem **O3/O7**.

---

## Part 2 — Open calibration experiments

Đây là phần cần làm để thay các con số đang đoán bằng số đo.

### O1 — `input_lead_us`: đo độ trễ thật (local và remote)

- **Vì sao mở:** lead là đại lượng **cảm nhận**, chưa đo trực tiếp. Hiện local 4000, audience 10000
  (vừa hạ từ 14500 bằng tai).
- **Chuẩn bị:** `TEST_metro_alt_500` (120 BPM); metronome tham chiếu (0.7).
- **Các bước (local):** với mỗi `--input-lead-ms` ∈ {0, 4, 8, 12, 16}:
  ```
  python -m main --song TEST_metro_alt_500 --timing-profile local-precise --input-lead-ms 8 --debug-csv
  ```
  Thu (overdub click, cách A) → tách onset → tính **offset = onset_game − click** trung bình.
- **Các bước (remote):** lặp lại nhưng máy B nghe; B chỉnh metronome 120 BPM và đánh giá lead nào
  trùng phách nhất (cách B), hoặc B thu + đo offset nếu B cũng dựng được click tham chiếu.
- **Quyết định:** lead local = giá trị làm offset_local ≈ 0; lead audience = giá trị làm offset_remote
  ≈ 0. **Hiệu hai cái = độ trễ mạng/replication thật** — đây là cơ sở duy nhất đáng tin cho phần
  lead dôi của audience. (Lead chỉ dịch trung bình, KHÔNG sửa được jitter ~12 ms của T3.)

### O2 — `chord_merge_window_us`: ngưỡng làm bẹt

- **Vì sao mở:** cửa sổ lớn gom nốt (mạch lạc) nhưng làm bẹt rải/arpeggio. Local 2500 vs audience
  5000 là đoán.
- **Chuẩn bị:** `TEST_rolled_chord_18` (4 phím cách 18 ms).
- **Các bước:** quét `--chord-merge-window-ms` ∈ {0, 5, 10, 15, 20, 30}:
  ```
  python -m main --song TEST_rolled_chord_18 --chord-merge-window-ms 10 --debug-csv
  ```
  Thu local; đếm xem mỗi block ra **4 onset rời** hay đã **gom thành 1**. (Có thể nhờ B nghe để xác
  nhận cảm giác "rải" còn không.)
- **Quyết định:** cửa sổ phải **nhỏ hơn** điểm bắt đầu gom. Đây là trần cho chord_merge của audience
  (liên quan trực tiếp O4).

### O3 — The real remote no-drop floor (sàn hold/gap audience theo số liệu)

- **Vì sao mở:** sàn audience (hold 20000 / repeat 24000) là ngoại suy; `local` (hold ~0 / repeat
  18000) **đôi khi mất nốt** remote. Ranh giới chưa đo.
- **Chuẩn bị:** máy B thu; `TEST_metro_same_200` + `TEST_repeat_staircase`.
- **Các bước:** giữ các tham số khác = local, nâng RIÊNG từng cái:
  - hold: `--hold-ms 0 → 8 → 12 → 16 → 20` (kèm `--min-hold-ms` tương ứng)
  - repeat_gap: `--repeat-release-gap-ms 18 → 20 → 22 → 24`
  Mỗi mức chạy N lần (gồm cả một lần mạng đông/kém). B đếm onset mất.
- **Quyết định:** hold/repeat_gap **nhỏ nhất mà 0 drop qua mọi lần** = sàn audience thật, thay ngoại
  suy ở T4/A.9. Chỉ nâng cái nào chặn drop; đừng nâng cái khác.

### O4 — ⭐ The local-vs-audience on-beat puzzle (trọng tâm)

- **Hiện tượng (báo cáo thực tế):** qua `local_precise`, máy người nghe thấy **chặt và đúng nhịp
  hơn** `audience_safe`, dù local **đôi khi mất nốt**. audience không mất nốt nhưng nghe lỏng/lệch
  nhịp.
- **Giả thuyết:** audience phá nhịp bởi **chord_merge (5000)** và **input_lead (10000)** (và phụ là
  hold dài); cái duy nhất chặn mất nốt là **hold/repeat_gap**. Liều thuốc đã overshoot.
- **Các bước (A/B một-biến):** giữ MỌI thứ = local, chỉ đẩy **một** cần gạt về giá trị audience mỗi
  biến; máy B chấm điểm "đúng nhịp" (1–5) và đếm drop cho từng biến:
  - V1: local + `--chord-merge-window-ms 5`
  - V2: local + `--input-lead-ms 10`
  - V3: local + `--hold-ms 20 --min-hold-ms 18`
  - V4: local + `--repeat-release-gap-ms 24`
  Nên làm **mù** (mục O4 ghi chú dưới): người chấm không biết đang nghe biến nào.
- **Quyết định:**
  - V1/V2 điểm nhịp **tệ hơn** → chord_merge/lead là thủ phạm phá nhịp → audience phải giữ chúng gần
    local.
  - V3/V4 **giảm drop** mà không hại nhịp → đó là cần gạt an toàn để nâng.
  - Tổng hợp: `audience_safe` đúng ≈ `local_precise` + mức nâng V3/V4 tối thiểu từ **O3**, chord_merge
    nhỏ, lead lấy từ **O1**. (Việc đã hạ lead 14500→10000 đi đúng hướng này.)
- **Ghi chú làm mù:** host phát cùng một bài dưới 2 cấu hình theo thứ tự ngẫu nhiên, đặt tên file thu
  trung tính (`A1.wav`, `A2.wav`); người chấm nghe xong mới mở bảng tra cấu hình. Lặp ≥3 vòng.

### O5 — The `<60 FPS` input-lead assumption (nhánh duy nhất chưa đo)

- **Vì sao mở:** dưới 60 FPS player nâng lead về `½ frame` theo lý thuyết render frame thô hơn tick
  ~60 Hz. EXP-2 chỉ đo 60 và 144.
- **Các bước:** khóa 30 FPS; dùng phương pháp metronome (O1) đo lead làm offset ≈ 0; so với lead ở
  60 FPS.
- **Quyết định:** nếu lead tối ưu @30 ≈ lead @60 → giả định nâng lead ở thấp FPS là sai, bỏ; nếu cao
  hơn rõ → giữ.

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
  so với 12 ms jitter game → sàn rộng càng vô ích.

### O8 — Remote chord integrity (mới)

- **Vì sao thêm:** mục tiêu lớn của audience là hợp âm nghe đủ/không vỡ ở máy khác; chưa đo riêng.
- **Các bước:** host phát `TEST_chords` (hợp âm 2..6 phím) qua local vs audience; B thu.
- **Đo:** mỗi hợp âm B nghe đủ số nốt không (đếm thành phần trong phổ tại mỗi onset), có "rattly/vỡ"
  không. So chord_merge nhỏ (local 2500) vs lớn (audience 5000).
- **Quyết định:** nếu chord_merge lớn KHÔNG cải thiện độ đủ nốt remote → bằng chứng nữa để hạ
  chord_merge audience (củng cố O4).

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

- Chỉ hạ một sàn **sau khi** O3 chứng minh 0 drop ở mức thấp hơn; không bao giờ hạ sàn chỉ để nhìn
  "nhanh hơn" (nguyên tắc §18/§20 của `timing-principles.md`).
- Khi một số đo đã chốt: cập nhật `config.py` (và `FrameTimingDefaults` nếu là tỷ lệ toàn cục) cùng
  dòng tương ứng trong `timing-principles.md` Appendix A, trích mã thí nghiệm (O1/O3/…) ở đây.
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

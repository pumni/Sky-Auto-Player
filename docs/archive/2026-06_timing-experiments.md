> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: File này chứa toàn bộ lịch sử các thí nghiệm timing cũ (O1 - O10.4) và các tham số cũ đã gỡ khỏi code.

# Timing Experiments — Proof and Open Calibration

Tài liệu này là hướng dẫn thực hành để **chứng minh** các giá trị timing đang được coi là chân
lý (Part 1) và để **đo chính xác** những con số còn đang đặt bằng tai hoặc ngoại suy — chủ yếu là
**sàn hold/min_hold remote thật**, cơ chế same-key ở mức game, và đặc tả jitter mạng (Part 2).

Công cụ: **Audacity** để thu và tách onset; một **máy thứ hai** đóng vai người nghe trong phòng
online. Mọi bước đều viết để bạn làm theo trực tiếp.

> Nguyên tắc tối cao: sự thật nằm ở **âm thanh game thu được**, không phải log. Nếu một kết quả đo
> ở đây mâu thuẫn với một "quy tắc kinh nghiệm" trong `timing-principles.md`, **kết quả đo thắng**.

> **Cập nhật 2026-06-06:** các ghi chú Phase G về "start-anchor fix" bên dưới là lịch sử. Contract
> hiện hành dùng completion-anchor và scheduler margin `release_latency_margin_us`; xem
> `completion-anchor-refactor-plan.md` và `timing-principles.md` §7 trước khi chạy lại gate.

> ⚠️ **CẬP NHẬT KIẾN TRÚC (2026-06) — đọc trước khi dùng file này.** Sau đợt audit + refactor 3 phase
> (xem [`timing-architecture-audit.md`](timing-architecture-audit.md)), **ba cần gạt đã bị XOÁ khỏi
> code**: `input_lead` (no-op kiến trúc — player tự sinh timeline, không có mốc tham chiếu ngoài),
> `chord_merge` (gần như không fire trên bài thật), `frame_align` (off mọi profile + vô nghĩa). Các
> thí nghiệm gắn với chúng (**O1, O2, O5**, và phần lớn **O4**) giờ **OBSOLETE** — giữ lại làm hồ sơ,
> không chạy nữa. Mọi flag `--input-lead-ms` / `--chord-merge-window-ms` / `--frame-align` đã biến
> mất. Sau audit corpus 2026-06-04, `release_gap` cũng bị xoá vì gần như không bind trên bài thật và
> làm profile semantics rối hơn giá trị nó mang lại. Vòng kiến trúc tiếp theo cũng đã xoá
> `repeat_release_gap` khỏi profile/CLI/runtime policy: O10 cho thấy nó là mechanism candidate
> nhưng không phải cần gạt production trong corpus/policy hiện tại. Mô hình local hiện còn
> `hold/min_hold`, trong đó `min_hold` là sàn visibility đã chứng minh. Phần CÒN MỞ thực sự:
> **O3, O6, O7, O9, O10** (+ O8 đã re-frame).

---

## 0. Setup and tooling (làm một lần)

### 0.1 Build the test songs

Chạy một lần để sinh các bài `songs/TEST_*.json`:

```
uv run python tests/make_test_song.py
```

Các bài cần dùng (đã có sẵn trong script):

| Song                     | Hình dạng                                        | Dùng cho                               |
| ------------------------ | ------------------------------------------------ | -------------------------------------- |
| `TEST_visibility`        | 15 nốt đơn, cách 700 ms                          | sàn hold/visibility (T1)               |
| `TEST_repeat_gap`        | 1 phím, interval = hold 24 ms + gap giảm 50→5 ms | cơ chế same-key lịch sử (T2)           |
| `TEST_repeat_gap_30`     | 1 phím, interval = hold 45 ms + gap 70→15 ms     | same-key mechanism sạch ở 30 FPS (O6)  |
| `TEST_repeat_gap_fine_*` | 1 phím, interval = hold 24 ms + gap 0→20 ms      | đo actual UP→DOWN gap sát biên (O10.4) |
| `TEST_repeat_staircase`  | 1 phím, interval giảm dần                        | drop same-key A/B (T2, T4)             |
| `TEST_metro_alt_200/120` | nhịp đều, **đan xen 2 phím**                     | jitter onset / cảm giác nhịp (T3, O4)  |
| `TEST_metro_same_200`    | nhịp đều, 1 phím                                 | survivability same-key remote (O3)     |
| `TEST_chords`            | hợp âm 2..6 phím                                 | chord remote (O8)                      |

Hai bài bổ sung (đã có sẵn trong script):

| Song                   | Hình dạng                             | Dùng cho                                               |
| ---------------------- | ------------------------------------- | ------------------------------------------------------ |
| `TEST_metro_alt_500`   | nhịp 120 BPM (500 ms), đan xen 2 phím | ~~O1/O5 (lead)~~ — giờ chỉ còn hữu ích cho O7 jitter   |
| `TEST_rolled_chord_18` | hợp âm "rải" 4 phím cách 18 ms        | ~~O2 (chord_merge)~~ **OBSOLETE** — chord_merge đã xoá |

> Hai bài này sinh ra cho các thí nghiệm đã OBSOLETE. Giữ trong script (vô hại); `TEST_metro_alt_500`
> còn dùng được cho O7. `TEST_rolled_chord_18` giờ chỉ minh hoạ: nốt cách 18 ms nay **luôn đi riêng**
> (không còn bị gom) — đúng behavior mới.

> `TEST_repeat_gap*` giờ là bài **mechanism/counterfactual/forensics**, không phải profile tuning
> target. Runtime không còn flag `--repeat-release-gap-ms`. Dùng `main` để phát actual authored gaps
> (`actual_up_gap = interval - hold` khi interval >= hold), và dùng
> `scripts/audit_repeat_gap.py --repeat-gap-ms` chỉ để hỏi "nếu vẫn giữ candidate gap này thì nó có
> bind không?".

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

# bài nhiều block (TEST_repeat_clean_*): tách IOI theo từng block (tránh std toàn bài bị phình
# bởi ~1.5s im lặng giữa các block):
uv run python tests/analyze_onsets.py labels.txt logs/playback_telemetry_XXXX.csv --per-block
```

Đọc kết quả:

- `[GAME] IOI mean/std/spread` — độ đều của nhịp thu được (std nhỏ = đều).
- `[SENDER AUDIT]` — `intended_down` / `sent_down` / `dropped_conflict` / `dropped_expired` /
  `suppressed_stale_up`. **Đây là tiền-gate, đọc TRƯỚC.** `[SENT]` chỉ tính các down **thật sự
  gửi** (`sent_scan_codes` khác rỗng); row `dropped_conflict`/`dropped_expired` bị loại, nếu không
  sẽ làm nhiễu baseline IOI và phần so audio.
- `[GATE]` — nếu `sent_down < intended_down` thì **audio KHÔNG hợp lệ làm ground truth**: các note
  thiếu đã mất *trước khi tới game*, nên không thể kết luận "game làm mất note". Phải đạt
  `sent_down == intended_down` rồi mới so onset.
- `[SENT] IOI std` — độ đều phía player gửi (phải rất nhỏ, ~<0.1 ms; chỉ trong cùng một block).
- `[GAME-only jitter] std` + `residuals` — phần dao động THUẦN do game (chỉ tin khi GATE OK). Nếu
  residuals có **dao động chu kỳ** (lặp +X/−X) → beating của tick nội bộ; nếu ngẫu nhiên → nhiễu
  thường.

> **Gate Tầng 2 (per-block, không phải toàn bài).** Ở `local_precise @144fps`, `min_hold = 6945 us`.
> Một block same-key chỉ là gate "phải 12/12 sent" khi **headroom = interval − min_hold > jitter
> dispatch thật của máy** (đo được ~2.5 ms spike trên máy dev). Vì vậy block **8 ms+** (headroom
> ≥ 1 ms) là gate hợp lệ; block **7 ms** (~55 us headroom) ở *đúng mép sàn* nên rớt là **tín hiệu
> tempo/profile (§timing-principles §18), KHÔNG phải lỗi anchor**. Đo thực tế (run1/run2, fix
> start-anchor): mọi block 8 ms+ đều 12/12; chỉ block 7 ms rớt 4 — khớp đúng dự đoán, tức fix
> validated ở mức sender cho mọi interval có headroom > jitter.

> **✅ PHASE G RESULT — start-anchor fix VALIDATED in-game (2026-06-06).** Probe ground-truth
> `TEST_repeat_clean_144` (interval 20/24/30/40/55/70 ms — sender-clean AND above the game
> re-trigger wall), `local_precise --fps 144`, 2 in-game runs + audio:
> - **Sender gate:** both runs `intended_down = sent_down = 72`, `dropped_conflict/expired/
>   suppressed_stale_up = 0` → audio is valid ground truth.
> - **Game onsets:** both runs **72/72**, all 6 blocks 12/12 — no lost notes, no lost block.
> - **Sender IOI per block:** 0.0027–0.0243 ms (≪ the 0.05–0.07 ms bar). Whole-song `[SENT] IOI std`
>   ≈ 382 ms is an artifact of the inter-block silence — read it `--per-block`.
> - **Game-only jitter per block:** 1.88–5.77 ms std — game/audio/detector side (consistent with
>   A.10), NOT sender/anchor.
> Conclusion: the start-anchor fix loses no same-key notes in real play for intervals with headroom
> above machine jitter; residual rhythm jitter is game-side. No profile-margin change warranted.

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
2. Khi cần tắt frame-aware để đo **bản chất của game**, truyền rõ `--fps 0`. Chỉ bỏ `--fps` là
   **không đủ**, vì player sẽ lấy `game_fps` từ `config.json` (hiện là 144). Khi đo logic FPS-aware
   của player, truyền rõ `--fps 30/60/144`. Mọi báo cáo phải ghi effective FPS/policy, không suy từ
   việc flag có xuất hiện hay không.
3. Khi quét actual same-key UP→DOWN gap (T2/O6/O10.4A): giữ hold **≥ 1 frame ở FPS test** để lỗi
   visibility không lẫn vào biến gap. Vì runtime không còn repeat-gap floor, actual gap đến từ
   authored interval: `actual_gap = next_down - previous_down - hold`.
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
nốt luôn phát đúng tại `source_time` (không dịch); nếu nghe lệch phách thì đó là bucket/phase
behavior của game (T3) hoặc trễ mạng, **không sửa được từ player**.

---

## Part 1 — Evidence ledger (chạy lại để kiểm chứng)

Không phải mục nào trong Part 1 còn là "confirmed truth" theo nghĩa production. Sau refactor, Part 1
là sổ cái bằng chứng:

| Mục | Mức tin cậy hiện tại                                   | Được dùng để quyết gì                                                                    |
| --- | ------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| T1  | **Strong production evidence**                         | `hold/min_hold` frame-visibility floor local.                                            |
| T2  | **Historical mechanism evidence**                      | Same-key UP→DOWN có cơ chế thật, nhưng không còn profile knob.                           |
| T3  | **Strong negative evidence against sender/lead fixes** | Không sửa onset scatter bằng input lead/frame align; internal bucket là game-side.       |
| T4  | **Limited remote negative evidence**                   | Sàn audience rộng chưa chứng minh cần trong mạng đã test; không đủ để chốt remote floor. |

Khi cập nhật `config.py`/`FrameTimingDefaults`, chỉ T1 đang là bằng chứng trực tiếp cho production
surface còn sống. T2/T3/T4 giữ vai trò giải thích và định hướng protocol cho Part 2.

### T1 — Game samples input once per render frame; visibility floor = 1 frame

- **Mục tiêu:** chứng minh nốt phải DOWN đủ lâu để bị lấy mẫu ở biên frame.
- **Chuẩn bị:** bài `TEST_visibility`; khóa FPS ngoài ở 30/60/144; ép player raw bằng `--fps 0`.
- **Các bước:** với mỗi FPS, quét hold giảm dần — chạy nhiều lần:
  ```
  uv run python -m main --song TEST_visibility --fps 0 --hold-ms 40 --min-hold-ms 40 --debug-csv
  uv run python -m main --song TEST_visibility --fps 0 --hold-ms 20 --min-hold-ms 20 --debug-csv
  uv run python -m main --song TEST_visibility --fps 0 --hold-ms 10 --min-hold-ms 10 --debug-csv
  ...
  ```
  (Phải đặt CẢ `--hold-ms` lẫn `--min-hold-ms` vì clamp hold hợp nhất lấy `max(min_hold, …)`.)
  Thu mỗi lần, đếm onset.
- **Đo:** hold nhỏ nhất mà vẫn đủ 15/15 onset.
- **✅ KẾT QUẢ ĐÃ ĐO (result.md):** @30 reliable 32 ms (31 rớt), @60 reliable 17 ms, @144
  reliable 7 ms đạt (1.0 frame) → sàn thật ≈ **1 frame** ở cả ba FPS (tuyến tính sạch). Hold dưới mép
  đăng ký theo xác suất.
- **Kết luận:** game đọc **state** theo frame, sàn visibility local của nốt đơn = **1.0 frame**.
  Đây là bằng chứng production mạnh cho `hold/min_hold` frame-aware. Giới hạn: T1 không tự chứng minh
  remote audience floor, chord integrity, hay compressed same-key edge case; các phần đó thuộc O3/O8
  và O10.4A nếu cần mechanism.

### T2 — Same-key UP→DOWN mechanism floor = max(~1.5 frame, ~18 ms) **[HISTORICAL / MECHANISM]**

> **Architecture correction:** runtime no longer exposes `repeat_release_gap`. T2 is kept as evidence
> that the game may need a visible UP state for same-key re-trigger under synthetic authored gaps.
> It is **not** a production profile knob.

- **Chuẩn bị:** `TEST_repeat_gap` (hold cố định 24 ms, gap giảm 50→5 ms) hoặc `TEST_repeat_staircase`;
  khóa FPS ngoài; hold giữ ≥ 1 frame ở FPS test.
- **Các bước:** khóa game lần lượt 60 rồi 144, ép player raw để actual gap đúng với authored block:
  `uv run python -m main --song TEST_repeat_gap --fps 0 --hold-ms 24 --min-hold-ms 24 --debug-csv`.
  Thu, đếm onset MỖI block (mỗi block 10 lần bấm cùng phím).
- **Đo:** block có gap nhỏ nhất mà vẫn đủ 10/10 onset.
- **✅ KẾT QUẢ ĐÃ ĐO (result.md):** @60 reliable **24 ms** (1.44 frame), @144 reliable **16 ms** (2.3
  frame). Ở 144 gap tin cậy >> 1.5 frame → có vẻ bị **tường thời gian cố định ~16–18 ms** chi phối
  (cùng họ với bucket/cadence nội bộ ở T3), không phải bội số frame thuần.
- **Kết luận:** mechanism lịch sử gợi ý `actual UP→DOWN gap ≥ max(1.5×frame, ~17–18 ms)`, nhưng
  production scheduler không còn field để enforce gap này. Bất kỳ quyết định giữ lại chỉ số này phải
  đi qua O10.4B counterfactual binding audit và một quyết định kiến trúc mới.

### T3 — Onset bucket-jumps are game-side/phase-dependent (player không sửa được)

> Hệ quả cũ "input lead không scale theo FPS" giờ MOOT (lead đã xoá). Nhưng phát hiện gốc vẫn đứng:
> sender sạch, còn audio onset có bucket-jump game-side theo pha. Đây là lý do nền tảng để KHÔNG cố
> "đẩy nốt cho đúng nhịp" từ player.

- **EXP-1 (phía gửi sạch):** `TEST_metro_alt_120` ở 60 và 144 với `--debug-csv`; phân tích CHỈ CSV.
  ✅ Đo: send-interval std ~0.036–0.052 ms, lateness p95 ~0.07–0.11 ms → **vấn đề nhịp KHÔNG do player**.
- **EXP-2 (jitter onset không giảm theo FPS):** `TEST_metro_alt_200`, ép player raw bằng `--fps 0`, khóa FPS
  ngoài 60 rồi 144; thu + `analyze_onsets.py labels.txt csv`.
  - ✅ **ĐÍNH CHÍNH so với kỳ vọng cũ:** jitter KHÔNG phải sàn cố định ~12–13 ms. Thực đo là **lưỡng
    cực / phụ thuộc pha**: có run sạch (residual std ~0.02 ms: 60fps-01, 144fps-02), có run dính
    **bucket-jump ~20 ms** (60fps-02 std 5.35 ms; 144fps-01 std 8.25 ms, IOI cụm 181/219 ms). Quan
    trọng: bucket-jump **không giảm khi lên 144** → không bám render frame.
- **Kết luận:** dữ liệu chứng minh rất mạnh rằng sender/player không tạo ra jitter này. Một số run có
  bucket-jump nội bộ khoảng **~20 ms** và hiện tượng này không biến mất chỉ vì render FPS lên 144.
  Cách gọi "~60 Hz internal tick" là **mô hình giải thích hợp lý**, không phải đo trực tiếp clock nội
  của game. Điều chắc chắn để quyết kiến trúc: không dịch đều (lead) hay snap frame (frame_align) nào
  chữa được scatter tương đối.

### T4 — Wide audience floors were not shown necessary in tested network (remote)

- **Chuẩn bị:** phòng online, máy B thu (0.5). A/B `audience_safe` vs `local_precise` (profile
  `audience_frame_test` đã bị xoá ở đợt rút gọn profile). LƯU Ý: sau refactor, audience_safe khác
  local_precise CHỈ ở sàn hold/min_hold (không còn khác lead/chord/release_gap/repeat_gap).
- **Các bước:** host phát `TEST_metro_same_200` rồi `TEST_repeat_staircase` lần lượt 2 profile; B thu
  từng lần.
- **Đo (trên WAV của B):** đếm onset mỗi bài; đo "valley drop" giữa 2 onset same-key (độ tụt
  envelope) xem có đủ down-up-down.
- **Kết quả đã có (2 phiên):** floor thấp pass gần ngang floor cao, không mất nốt hệ thống.
- **Kết luận:** dưới mạng đã test, sàn rộng **chưa chứng minh là cần**. Đây là negative evidence có
  phạm vi hẹp, không phải proof rằng audience floors redundant. Chưa stress mạng xấu → xem **O3/O7**.

---

## Part 2 — Open calibration experiments

Phần này thay các con số đang đoán bằng số đo. **Trạng thái sau refactor:** O1/O2/O5 đã OBSOLETE
(knob bị xoá); O4 re-frame; còn mở thực sự = **O3, O6, O7, O8, O9, O10**.

### Reliability audit — đọc trước khi chạy Part 2

Không phải thí nghiệm nào trong Part 2 cũng có cùng sức nặng. Tạm phân cấp như sau:

| Mục       | Có thể tin tới mức nào?                         | Lý do chính                                                                           |
| --------- | ----------------------------------------------- | ------------------------------------------------------------------------------------- |
| O9        | **Gate bắt buộc**, chưa phải gameplay truth     | Nó chỉ chứng minh detector đếm đúng trong điều kiện thu hiện tại.                     |
| O3        | **Production decision được**, nếu đủ run        | Đo trực tiếp remote no-drop, nhưng phải randomize và có nhiều phiên mạng.             |
| O7        | **Characterization**, không tự chốt hold        | IOI jitter mô tả mạng/game, nhưng drop/visibility vẫn phải do O3/O8 quyết.            |
| O8        | **Supporting evidence**                         | Chord đủ/vỡ ở remote khó đo tự động; dùng để củng cố O3/O4, không chốt một mình.      |
| O6/O10.4A | **Mechanism only**                              | Same-key gap là cơ chế game/synthetic, không còn profile field.                       |
| O10.2     | **Actionable only nếu raw/no-FPS là mode thật** | Nếu production luôn biết FPS/config FPS, fallback raw không nên được ưu tiên quá mức. |
| O10.5     | **Engine benchmark**                            | Chọn global sleeper threshold; không phải gameplay floor.                             |
| O10.6     | **Blocked**                                     | Thiếu wall-clock focus instrumentation.                                               |

Quy tắc chung: một kết quả chỉ được gọi là “sự thật production” khi nó vừa có **runtime surface còn
sống**, vừa **đổi được schedule**, vừa **được đo trên bài/corpus đại diện**. Nếu thiếu một trong ba,
ghi là mechanism, counterfactual, hoặc supporting evidence.

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

### O3 — The real remote no-drop floor (sàn hold/min_hold audience theo số liệu) — **CÒN MỞ**

> **Trạng thái (result.md):** user **gác có chủ đích** để làm chuẩn local trước ("xử lý tốt local thì
> remote cũng tốt"). Sau refactor, audience_safe khác local_precise chỉ ở sàn → đây là nơi DUY NHẤT
> còn lý do tồn tại của audience_safe; cần O3 để chứng minh sàn hold/min_hold nào thật sự cần.

- **Vì sao mở:** sàn audience hold/min_hold là ngoại suy; `local_precise` (visibility 1.05 frame)
  **đôi khi mất nốt** remote. Ranh giới chưa đo.
- **Chuẩn bị:** máy B thu; `TEST_metro_same_200` + `TEST_repeat_staircase`.
- **Các bước:** giữ các tham số khác = local, nâng RIÊNG hold/min_hold. Không dùng `--hold-ms 0` làm
  candidate production: nó là stress/negative control, không phải floor hợp lý. Ở 144 FPS nên quét:
  - baseline profile `local_precise` không override;
  - `--hold-ms 7.3 --min-hold-ms 7.3` hoặc mức xấp xỉ 1 frame nếu muốn explicit;
  - `--hold-ms 8 --min-hold-ms 8`;
  - `--hold-ms 12 --min-hold-ms 12`;
  - `--hold-ms 16 --min-hold-ms 16`;
  - `--hold-ms 20 --min-hold-ms 20`.
    Mỗi mức chạy theo thứ tự randomized, ít nhất 3 run trong mạng tốt và 3 run trong mạng xấu/đông.
    Mọi command O3 phải ghi rõ `--timing-profile local-precise --fps 144 --debug-csv` cộng với override
    hold/min_hold tương ứng; không dựa vào default profile.
- **Không sweep repeat-gap:** runtime không còn `--repeat-release-gap-ms`; nếu same-key remote vẫn
  drop, kiểm authored interval/min_hold/tempo trước. `audit_repeat_gap.py --repeat-gap-ms` chỉ là
  counterfactual, không tạo playback khác.
- **Đo:** tách theo bài. `TEST_metro_same_200` đo repeat survivability vừa phải; `TEST_repeat_staircase`
  là stress same-key và không được dùng một mình để kết luận audience production. Ghi `eligible notes`,
  `dropped notes`, run id, profile/flags, FPS effective, mạng tốt/xấu.
- **Quyết định:** hold/min_hold nhỏ nhất mà 0 drop qua mọi run đại diện = sàn audience thật. Nếu chỉ
  pass trong mạng tốt hoặc chỉ pass bài synthetic, kết luận là **INCONCLUSIVE / stress-only**.

### O4 — The local-vs-audience on-beat puzzle — **PHẦN LỚN ĐÃ GIẢI (bằng suy luận sau refactor)**

- **Hiện tượng (báo cáo thực tế):** qua `local_precise`, máy người nghe thấy **chặt và đúng nhịp
  hơn** `audience_safe`, dù local **đôi khi mất nốt**.
- **Giả thuyết cũ đã BỊ BÁC một phần:** nghi phạm chính từng là **chord_merge** + **input_lead**.
  Cả hai giờ đã chứng minh vô hại/no-op và **đã xoá**. → Sau refactor, onset của local và audience
  **giống hệt nhau** (cùng = `source_time`, không còn lead/chord làm khác). Vậy audience **không thể**
  "lạc nhịp tương đối" so với local từ phía player.
- **Cái còn lại có thể gây "lỏng":**
  1. **hold dài hơn** của audience (20000 vs 1.05f) → key chồng lấn nhiều hơn = cảm giác "mushy", dù
     KHÔNG dịch onset;
  2. **trễ + jitter mạng** + bucket-jump ~20 ms của game (T3) — run-to-run, không phải local-vs-audience;
- **Còn cần đo (nếu quay lại nhánh remote):** A/B một-biến giữa local và audience (làm mù, máy B
  chấm "đúng nhịp" 1–5 + đếm drop):
  - V1: local + `--hold-ms 20 --min-hold-ms 18` (kiểm "mushy" do hold dài)
- **Độ tin cậy:** đây là perceptual A/B, không phải chứng minh cơ chế. Nó chỉ đáng tin nếu thứ tự run
  được randomize, người chấm không biết cấu hình, và kết quả lặp lại qua nhiều bài.
- **Quyết định:** nếu V1 làm điểm nhịp tệ hơn mà không giảm drop trong O3/O8 → hold dài là nghi phạm
  chính của cảm giác "lỏng", giữ audience hold sát local. Nếu chỉ điểm nghe khác mà không có objective
  drop/chord metric, ghi là **preference**, không đổi config một mình.
- **Ghi chú làm mù:** host phát cùng bài dưới 2 cấu hình thứ tự ngẫu nhiên, tên file trung tính
  (`A1.wav`, `A2.wav`); người chấm nghe xong mới tra cấu hình. Lặp ≥3 vòng.

### O5 — ~~The `<60 FPS` input-lead assumption~~ **[OBSOLETE — knob đã xoá]**

> input_lead bị xoá; nhánh `<60` nâng lead về `½ frame` (`input_lead_min_frame_ratio`) cũng đã gỡ ở
> Phase 1. Không còn lead để đo ở FPS thấp. (Sàn hold/gap ở 30 FPS vẫn cần đo — xem **O6**.)

### O6 — Clean 30 FPS same-key mechanism point (mục A.8 còn treo)

- **Vì sao mở:** T2 mới có 60 và 144; mechanism model dự đoán actual UP→DOWN gap ~46 ms @30
  (hold phải ≥ 42 ms) nhưng chưa đo.
- **Các bước:** dùng `TEST_repeat_gap_30`, vì bài này được authored với hold 45 ms nên actual gap
  không bị đổi khi hold đủ visibility @30. Khóa game 30 FPS, ép player raw:
  `uv run python -m main --song TEST_repeat_gap_30 --fps 0 --hold-ms 45 --min-hold-ms 35 --debug-csv`.
  Đếm onset mỗi block.
- **Độ tin cậy:** một run chỉ có 9 eligible transitions cho mỗi gap level, quá ít để gọi là
  "100%-tin-cậy". Muốn ghi reliable boundary, cần nhiều run/variant để đạt ít nhất 100 eligible
  transitions ở gap được gọi là pass.
- **Quyết định:** ghi gap @30 là **mechanism note** thôi. Không dùng O6 để tune production profile nếu
  O10.4B vẫn không có schedule-changing binding trên corpus.

### O7 — Network delay & jitter characterization (mới)

- **Vì sao thêm:** O3 cần biết mạng tệ đến đâu mới đặt được biên an toàn cho sàn. Cần đặc tả phân bố
  trễ/jitter của replication, không chỉ "pass/fail".
- **Các bước:** host phát `TEST_metro_alt_200` (nhịp đều). Máy A thu local (qua loopback) ĐỒNG THỜI
  máy B thu remote. Vì 2 đồng hồ khác nhau, dùng **IOI**, không dùng absolute offset. Chạy với
  `--timing-profile local-precise --fps 144 --debug-csv` trừ khi đang cố ý đo profile khác.
- **Control bắt buộc:** A và B phải đếm cùng số onset, cùng thứ tự nốt. Nếu B miss/extra onset thì run
  không còn là jitter characterization; chuyển nó sang O3 drop evidence.
- **Đo:** căn index onset theo thứ tự nốt, rồi so std(IOI) của B với std(IOI) của A. Công thức
  `jitter_mạng ≈ sqrt(max(0, std_B² − std_A²))` chỉ là gần đúng khi nhiễu độc lập; báo cáo nó là
  estimate, không phải floor.
- **Quyết định:** O7 chỉ mô tả replication jitter để giải thích cảm giác remote. Không tự biến jitter
  thành hold margin; hold/min_hold audience vẫn phải do O3/O8 pass/fail quyết.

### O8 — Remote chord integrity — **RE-FRAME (không còn chord_merge để so)**

- **Vì sao giữ:** mục tiêu lớn của audience là hợp âm nghe đủ/không vỡ ở máy khác — vẫn là câu hỏi
  hợp lệ, ĐỘC LẬP với chord_merge (knob đó đã xoá). Giờ chord = các nốt **cùng timestamp** được gửi
  trong một SendInput (gom ở event-grouping cuối).
- **Các bước:** host phát `TEST_chords` qua local vs audience và một biến hold explicit
  (`--timing-profile local-precise --fps 144 --hold-ms 20 --min-hold-ms 18`) theo thứ tự randomized;
  B thu. Mọi run phải ghi profile/flags thật, vì default CLI là `balanced`.
- **Đo:** đây là phần khó nhất để tự động hóa. Nếu nhạc cụ/pitch của từng phím đủ phân biệt, đo phổ
  quanh từng onset và đếm đủ thành phần kỳ vọng. Nếu không phân biệt được phổ, dùng blind rating
  "đủ/vỡ/rattly" và ghi rõ là subjective. Mỗi chord size cần nhiều lần lặp; `TEST_chords` hiện chỉ có
  một chord cho mỗi size nên chỉ là smoke test, chưa đủ để gọi reliable.
- **Quyết định:** nếu hold dài KHÔNG cải thiện độ đủ nốt remote → thêm bằng chứng để giữ audience hold
  sát local (củng cố O3/O4). Một mình O8 không chốt config.

### O9 — Onset-detector calibration (control — làm trước khi tin số liệu)

- **Vì sao thêm:** mọi kết luận dựa trên Label Sounds; phải biết detector đếm đúng.
- **Chuẩn bị:** dùng cùng nhạc cụ, âm lượng game, input device, Audacity setup như các thí nghiệm sau.
  Khóa FPS theo mục tiêu đo; với detector-control nên bắt đầu ở 60 FPS.
- **Các bước chi tiết:**
  1. Mở Audacity WASAPI loopback, Project Rate 48000 Hz, Mono.
  2. Bấm Record, để im lặng khoảng 1 giây.
  3. Chạy:
     ```
     uv run python -m main --song TEST_visibility --timing-profile local-precise --fps 144 --hold-ms 30 --min-hold-ms 30 --debug-csv
     ```
  4. Dừng record sau khi bài kết thúc, lưu WAV với tên có profile/run, ví dụ
     `Test-visibility-run1-local.wav`.
  5. Chạy **Analyze -> Label Sounds**. Bắt đầu với threshold khoảng `-33 dBFS`, min silence
     `0.075 s`, Label type = start/before sound.
  6. Export labels rồi chạy:
     ```
     uv run python tests/analyze_onsets.py labels.txt logs/playback_telemetry_XXXX.csv
     ```
- **Đo:** số nhãn phải = 15, IOI mean gần 700 ms. Std có thể cao nếu game bucket-jump theo T3, nhưng
  detector vẫn pass nếu đếm đúng 15/15 và onset nằm đúng từng nốt. Chạy ít nhất 3 run cho cùng
  threshold; một run pass chỉ là smoke test.
- **Kết quả control đã ghi nhận:**

  | Field                     | Value                            |
  | ------------------------- | -------------------------------- |
  | Control file              | `Test-visibility-run1-local.wav` |
  | Expected                  | 15 onset, IOI 700 ms             |
  | Detected                  | **15/15**                        |
  | IOI mean/std              | **699.942 / 10.585 ms**          |
  | Chosen detector threshold | **-33 dBFS**                     |
  | Chosen min silence        | **0.075 s**                      |
  | Decision                  | **PASS**                         |

- **Lưu ý quan trọng:** cùng ngưỡng năng lượng từ O9 được dùng để ổn định detector, nhưng với
  `TEST_repeat_gap` / `TEST_repeat_staircase` **không dùng nguyên `min_silence=0.075 s` để đếm onset
  riêng**, vì chính mục tiêu của hai bài này là kiểm same-key repeat rất nhanh. Báo cáo mechanism
  same-key phải dùng **attack-lobe count trong từng block** thay cho label-count kiểu bài nốt rời.
- **Quyết định:** dùng threshold `-33 dBFS` làm điểm khởi đầu cho cùng nhạc cụ/âm lượng, không gọi là
  universal truth. Nếu đổi nhạc cụ, âm lượng, bài same-key nhanh, hoặc remote audio có noise khác,
  chạy lại O9/mini-O9 trước khi tin O10/T1/T2.

### O10 — Field-level audit cho `local_precise` — **CÒN MỞ CHO CÁC SỐ CHƯA CÓ THỰC NGHIỆM RIÊNG**

> Mục tiêu của mục này là trả lời trực tiếp câu hỏi: trong một profile kiểu `local_precise`, field nào
> đã đứng trên bằng chứng thực đo, field nào chỉ là biên an toàn / suy luận / hạ tầng chưa có thí
> nghiệm riêng. Khi một field đã được đo xong, cập nhật dòng tương ứng ở bảng này và Appendix A của
> `timing-principles.md`.

#### O10.0 Protocol gate — bắt buộc trước mọi kết luận

O10 chỉ có giá trị khi phân biệt rõ bốn câu hỏi khác nhau:

1. **Mechanism:** game có thật sự cần floor này trong synthetic stress case không?
2. **Reachability/binding:** code path có thể thay đổi schedule với policy hiện tại, và có gặp bài
   thật ở tempo production không?
3. **Production decision:** lợi ích có đủ lớn để giữ field/profile differentiation không?
4. **Runtime surface:** field đó có còn tồn tại trong profile/CLI/policy không?

Một mechanism có thể thật nhưng binding rate bằng 0; khi đó không được gọi field là production
lever. Nếu runtime surface đã bị xoá, mọi đo đạc còn lại chỉ là mechanism/counterfactual, không phải
tuning guide cho profile.

Trước mỗi run:

- Dùng `--fps 0` để ép raw mode; dùng `--fps N` để ép frame-aware. Không dùng việc “bỏ `--fps`” làm
  bằng chứng raw mode vì config có thể tự điền FPS.
- Ghi **effective** `fps/frame_us/hold_us/min_hold_us` và actual schedule.
  Giá trị CLI chỉ là yêu cầu đầu vào, không phải bằng chứng về gap/hold thực tế.
- Dùng `scripts/audit_repeat_gap.py --song ... --hold-ms ... --min-hold-ms ... --repeat-gap-ms ...`
  chỉ như **counterfactual** cho same-key experiment; tool in effective policy, hypothetical
  compression band và actual runtime gaps.
- Với same-key block có `N` nốt, số lần thật sự kiểm re-trigger là `N-1`; nốt đầu chỉ là control.
- Chạy thứ tự mức theo vòng ngẫu nhiên/đảo thứ tự để tránh confound nhiệt, tải nền và “cuối bài”.
- Báo cáo từng run rồi mới lấy median/range; không pool toàn bộ event để che một run xấu.
- Với zero-failure test, ghi số **eligible transitions**. Quy tắc gần đúng: 0 lỗi trên `n` transition
  chỉ cho upper bound lỗi 95% khoảng `3/n`. Ba run của block 10 nốt chỉ có 27 transition, vẫn cho
  upper bound khoảng 11%; chưa đủ để gọi là production-reliable.
- Kết quả theo protocol cũ sai effective timing phải giữ như lịch sử nhưng đánh dấu **INVALID FOR
  DECISION**, không trộn vào bảng chốt mới.

#### O10.1 Ma trận uy tín hiện tại

| Field / mechanism                     | Runtime surface hiện tại                     | Trạng thái bằng chứng                                                                               | Việc còn cần nếu muốn gọi là "đã chốt"                                                                           |
| ------------------------------------- | -------------------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `hold_frames`                         | **CÒN SỐNG** trong profile                   | T1 chứng minh visibility floor ≈ 1 frame ở 30/60/144 FPS. `1.05` là margin local-sharp rất sát sàn. | Regression T1 với profile hiện tại ở 30/60/144, mỗi FPS ≥3 run, xác nhận 15/15 onset bằng detector O9.           |
| `hold_floor_us = 0`                   | **CÒN SỐNG** trong profile                   | T1 cho thấy local visibility là frame-relative; không thấy fixed-ms floor vì 7 ms vẫn đủ ở 144 FPS. | Regression sau game update/đổi nhạc cụ. Remote thuộc O3/O8.                                                      |
| `hold_unframed_us`                    | **CÒN SỐNG** raw fallback                    | Fallback khi effective FPS = 0, không phải floor đã đo.                                             | O10.2: explicit `--fps 0`, so 18/20/22/24 ms, chọn mức nhỏ nhất 0 drop.                                          |
| `min_hold_frames`                     | **CÒN SỐNG** trong profile                   | Cùng bản chất visibility như hold; compressed same-key hold vẫn phải sống ≥ 1 frame.                | Nếu tách khỏi `hold_frames`, đo bài same-key có compression; luôn giữ `min_hold_frames <= hold_frames`.          |
| `min_hold_floor_us = 0`               | **CÒN SỐNG** trong profile                   | Cùng lý do với `hold_floor_us = 0`; local visibility không có fixed-ms floor trong T1.              | O3/O8 nếu muốn dùng cho online audience; local regression sau game update.                                       |
| `min_hold_unframed_us`                | **CÒN SỐNG** raw fallback                    | Raw/no-FPS fallback, không phải số đo độc lập.                                                      | Đo cùng O10.2; nếu hold/min_hold tách nhau, phải có bài compression.                                             |
| actual same-key UP→DOWN gap mechanism | **KHÔNG CÒN** profile/CLI/runtime knob       | T2 gợi ý game mechanism có thật, nhưng O10.4B/corpus cho thấy không phải production lever.          | O6/O10.4A chỉ ghi hồ sơ mechanism/counterfactual; không dùng để tune profile nếu không reintroduce architecture. |
| counterfactual repeat-gap binding     | **Chỉ còn audit script**, không đổi playback | Corpus thật 0 schedule-changing positive interval tới tempo 3.0x.                                   | Chạy lại sau corpus lớn hơn/game update. Nếu vẫn 0, tiếp tục bỏ.                                                 |
| `spin_threshold_us`                   | **CÒN SỐNG** runtime infra override          | Tác động sleeper lateness/CPU, không tác động floor game.                                           | O10.5 randomized benchmark; mục tiêu chọn global SleepPolicy, không profile semantic.                            |
| `focus_restore_grace_us`              | **CÒN SỐNG** runtime safety                  | Thiếu observability wall-clock để đo đúng.                                                          | O10.6 thêm instrumentation rồi chọn global safety value.                                                         |

#### O10 priority — thứ tự nên làm

1. **Chạy O10.2** nếu raw/no-FPS fallback thực sự là mode production cần hỗ trợ. Đây là thí nghiệm
   gameplay còn actionable ngay với tooling hiện tại.
2. **Chạy O3/O8** để quyết định audience-safe hold/min_hold remote thật sự.
3. **Chạy O10.5** sau khi có cách tag condition và đo process CPU; kết quả phải chọn một global value.
4. **Không chạy O10.6 để chốt số** trước khi thêm focus wall-clock instrumentation.
5. **Chỉ chạy O6/O10.4A** nếu muốn hoàn thiện hồ sơ game-mechanism same-key; không dùng kết quả đó
   để tune production profile nếu không reintroduce một kiến trúc repeat-gap mới.

#### O10.2 Đo `hold_unframed_us` / `min_hold_unframed_us` raw fallback

- **Mục tiêu:** chốt fallback khi effective FPS thật sự bằng 0/unknown.
- **Giới hạn:** `TEST_visibility` chỉ đo effective hold của nốt đơn. Nó không thể độc lập chứng minh
  một `min_hold_unframed_us` thấp hơn hold. Với `local_precise`, cách sạch nhất là chọn một fallback
  thống nhất cho cả hold/min_hold; nếu muốn tách hai số, phải thêm compressed same-key test riêng.
- **Chuẩn bị:** chạy O9 trước; khóa game ở FPS thực tế bạn hay dùng nhưng truyền rõ `--fps 0` cho
  player; dùng `TEST_visibility`. Mỗi mức cần ít nhất 3 run riêng.
- **Các bước chi tiết:**
  1. Đặt tên run trước khi thu: `visibility-raw-hold-18-run1.wav`,
     `visibility-raw-hold-18-run2.wav`, ...
  2. Trong Audacity, dùng threshold O9 (`-33 dBFS`) và min silence O9 (`0.075 s`) vì bài này có nốt
     cách 700 ms.
  3. Chạy từng mức hold/min_hold. CLI nhận millisecond thập phân, nên có thể đo sát hơn nếu cần:

     uv run python -m main --song TEST_visibility --fps 0 --hold-ms 18 --min-hold-ms 18 --debug-csv
     uv run python -m main --song TEST_visibility --fps 0 --hold-ms 20 --min-hold-ms 20 --debug-csv
     uv run python -m main --song TEST_visibility --fps 0 --hold-ms 22 --min-hold-ms 22 --debug-csv
     uv run python -m main --song TEST_visibility --fps 0 --hold-ms 24 --min-hold-ms 24 --debug-csv

  4. Nếu 18/20/22/24 đều pass, đo thêm vùng sát hơn:
     uv run python -m main --song TEST_visibility --fps 0 --hold-ms 16.5 --min-hold-ms 16.5 --debug-csv
     uv run python -m main --song TEST_visibility --fps 0 --hold-ms 17.0 --min-hold-ms 17.0 --debug-csv
     uv run python -m main --song TEST_visibility --fps 0 --hold-ms 17.5 --min-hold-ms 17.5 --debug-csv

  5. Export labels và phân tích từng run:

     uv run python tests/analyze_onsets.py labels.txt logs/playback_telemetry_XXXX.csv

- **Đo:** mỗi run ghi `expected=15`, `detected`, IOI mean/std, và ghi có onset nào thấp/mờ bất thường
  không. Một mức chỉ pass nếu **mọi run đều 15/15**.
- **Quyết định:** mức nhỏ nhất 15/15 ổn định là fallback local raw. Nếu một mức miss ở bất kỳ run nào,
  không dùng mức đó làm default. Nếu không đo, giữ fallback bảo thủ và không gọi nó là "đã kiểm nghiệm".
- **Mẫu báo cáo:**
  ```
  O10.2 raw hold fallback | FPS lock: __ | player --fps 0
  Level: __ ms | Runs: __/__
  Detector: threshold -33 dBFS, min silence 0.075 s
  Run results: 15/15, 15/15, 15/15
  IOI mean/std range: __ / __ ms
  Decision: PASS/FAIL, chosen fallback = __ ms
  ```

#### O10.3 ~~Đo riêng `release_gap_us`~~ **[REMOVED]**

`release_gap_us` đã bị xoá khỏi code/profile/CLI sau audit timeline trên corpus thật. Audit cho thấy
current-code binding gần như bằng 0 và ngay cả guard-band rộng hơn cũng chỉ bind một số event rất nhỏ.

Không chạy O10.3 nữa; nếu dense/online mất nốt, kiểm `min_hold`, profile an toàn hơn, hoặc giảm
tempo.

#### O10.4 Same-key mechanism và counterfactual binding audit

O10.4 không còn là thí nghiệm để tune profile field. Runtime đã xoá `repeat_release_gap`. Mục này
chỉ còn hai mục đích:

1. đo hồ sơ game-mechanism: actual same-key UP→DOWN gap thấp đến đâu thì còn re-trigger;
2. chạy counterfactual binding audit: nếu giả sử còn candidate gap, corpus thật có bind không?

##### O10.4A — Mechanism: game cần actual same-key UP→DOWN gap bao nhiêu?

- **Chuẩn bị:** sinh lại test songs, khóa game 144 FPS, dùng lần lượt
  `TEST_repeat_gap_fine_a/b/c`. Các bài fine có authored gap thật gồm `0/1/2/3/5/8/11/14/17/20 ms`,
  mỗi block 20 nốt = **19 eligible re-trigger transitions**, và ba thứ tự block khác nhau.
- **Policy bắt buộc:** truyền rõ `--fps 144 --hold-ms 24 --min-hold-ms 7`. Hold 24 ms khớp cách bài
  được authored; min_hold 7 ms vẫn qua visibility floor @144. Actual gap đến từ bài:
  `actual_gap = next_down - previous_down - 24ms` khi interval ≥ hold.
- **Scouting:** chạy audit counterfactual gap 0 và phát cả ba variant để tìm vùng fail/pass thô:
  ```
  uv run python scripts/audit_repeat_gap.py --song songs/TEST_repeat_gap_fine_a.json --profile local-precise --fps 144 --hold-ms 24 --min-hold-ms 7 --repeat-gap-ms 0
  uv run python -m main --song TEST_repeat_gap_fine_a --fps 144 --hold-ms 24 --min-hold-ms 7 --debug-csv
  uv run python -m main --song TEST_repeat_gap_fine_b --fps 144 --hold-ms 24 --min-hold-ms 7 --debug-csv
  uv run python -m main --song TEST_repeat_gap_fine_c --fps 144 --hold-ms 24 --min-hold-ms 7 --debug-csv
  ```
- **Confirmation:** không còn candidate floor trong runtime. Xác nhận bằng cách tập trung vào các
  authored gap quanh boundary tìm được (ví dụ 11/14/17/20 ms), đảo thứ tự bài variant/run để tránh
  confound.
- **Đo:** attack-lobe count từng block; không dùng `min_silence=75 ms`. Ghi `onsets/20`,
  `eligible transitions=19`, số failed re-trigger, và actual gap từ schedule/CSV. Nếu detector không
  tách được attack thì run là **INCONCLUSIVE**, không phải FAIL.
- **Tiêu chí mechanism:** một actual gap được gọi là reliable khi 0 failed re-trigger trên ít nhất
  100 eligible transitions cùng actual gap qua nhiều variant/run. Với 0 lỗi/100, upper bound lỗi 95%
  vẫn khoảng 3%. Kết quả này không tự động tạo profile field mới.

##### O10.4B — Counterfactual binding: nếu còn candidate gap thì có bind bài thật không?

Chạy corpus audit ở tempo production thường dùng và một mức stress hợp lý:

```
uv run python scripts/audit_repeat_gap.py --profile local-precise --fps 144 --tempo-scale 1.0 --repeat-gap-ms 17
uv run python scripts/audit_repeat_gap.py --profile local-precise --fps 144 --tempo-scale 1.25 --repeat-gap-ms 17
uv run python scripts/audit_repeat_gap.py --profile local-precise --fps 144 --tempo-scale 1.25 --repeat-gap-ms 0
```

Phải tách ba vùng counterfactual:

- **pressure:** `interval < hold + gap`; normal hold không để lại requested gap;
- **schedule-changing compression band:** `min_hold + gap <= interval < hold + gap`;
- **under candidate cycle:** `interval < min_hold + gap`; nếu architecture cũ còn tồn tại thì đây là
  vùng không enforce được requested gap trong degraded mode.

Nếu `hold == min_hold`, compression band rỗng và candidate repeat gap không thể thay đổi degraded
playback schedule. Đây chính là trạng thái của mọi profile frame-aware hiện tại. Báo cáo vẫn phải
tách positive interval khỏi zero-time duplicate/chord và ghi `duplicate_note_count`.

- Nếu mechanism có thật nhưng compression band rỗng hoặc schedule-changing binding ≈0: tiếp tục
  không giữ production field.
- Chỉ cân nhắc reintroduce architecture nếu vừa có mechanism fail ở actual gap thấp, vừa có corpus
  production với schedule-changing compression band đáng kể, vừa chứng minh audio remote/local tốt hơn.

##### Kết quả O10.4 cũ

**INVALID FOR DECISION.** Các run cũ dùng `hold_ms=10` trên bài authored với hold 24 ms. Vì vậy
floor CLI 0/15/17/18 đều để lại actual minimum gap khoảng 19 ms; chúng chỉ chứng minh 19 ms pass,
không phân biệt được 0, 17 hay 18 ms. Không dùng các run đó để chốt floor.

Mẫu báo cáo mới:

```
O10.4A @144 | song variant: __ | effective hold/min: __/__
Block authored gap: __ | actual scheduled UP→DOWN gap: __ | onsets: __/20
Eligible re-trigger transitions: 19 | failures: __ | cumulative same-gap exposure: __
Decision mechanism only: PASS/FAIL/INCONCLUSIVE

O10.4B | profile/fps/tempo: __ | candidate repeat gap: __
Positive pressure intervals: __/__ | compression-band intervals: __ | compressed holds: __
Under candidate cycle: __ | duplicate_note_count: __ | min actual same-key up gap: __
Decision production: keep removed / investigate reintroducing architecture
```

#### O10.5 Đo `spin_threshold_us`

- **Mục tiêu:** chọn một global sleeper threshold cân bằng sent-side lateness và CPU. Đây không phải
  gameplay floor và không nên khác theo timing profile.
- **Giới hạn tooling hiện tại:** summary JSON có lateness nhưng chưa tag `spin_threshold_us` và chưa
  đo process CPU time. Vì vậy phải lưu run IDs/condition riêng; quan sát Task Manager chỉ là phụ,
  không đủ để chốt CPU trade-off.
- **Thiết kế:**
  1. Giữ cố định power mode, FPS, profile và app nền. Dùng `TEST_metro_alt_120`.
  2. Mỗi vòng chạy đủ 4 mức theo thứ tự đảo/ngẫu nhiên, không chạy hết 0 rồi mới tới 1200.
  3. Phase A dùng `--dry-run` để isolate sleeper; Phase B xác nhận hai mức tốt nhất bằng SendInput thật.
  4. Sau mỗi run, ghi run ID và threshold hoặc đưa cặp CSV/summary vào thư mục condition riêng.
  5. Ít nhất 7 run/mức; báo cáo median và worst-run, không pool mọi event.
     ```
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 0 --debug-csv --dry-run
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 500 --debug-csv --dry-run
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 800 --debug-csv --dry-run
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 1200 --debug-csv --dry-run
     ```
- **Đo:** down-event IOI std, lateness p95/p99, số event >1/>2 ms, worst-run max, process CPU time.
  Không dùng WAV/game audio để chốt threshold này.
- **Quyết định:** chọn mức thấp nhất có median p95/p99 tương đương mức tốt nhất và không có worst-run
  regression đáng kể. Nếu chưa có process CPU measurement, kết quả chỉ là sent-side benchmark,
  chưa đủ để chốt global threshold.
- **Mẫu báo cáo:**
  ```
  O10.5 spin threshold | Phase dry/live | Runs per level: __ | randomized rounds: yes/no
  __ us: median down-IOI std __; median p95/p99 __/__ us; worst max __; CPU time __
  Decision: global spin_threshold_us = __ / INCONCLUSIVE (missing CPU)
  ```

#### O10.6 Đo `focus_restore_grace_us`

- **Trạng thái:** **BLOCKED BY OBSERVABILITY — không chạy protocol thủ công cũ để chốt số.**
- **Lý do:** engine pause playback timeline khi mất focus, đợi grace, rồi cộng cả focus-pause + grace
  vào `pause_time_us`. Telemetry `actual_us/lateness_us` dùng playback time đã trừ khoảng đó, nên CSV
  hiện tại không thể cho biết wall-clock grace hay khoảng từ “focus active” tới SendInput đầu tiên.
  Alt-Tab bằng tay “ngay trước một nốt” còn trộn thêm phase còn lại của nốt, nên kết quả không isolate
  grace.
- **Instrumentation cần có trước:** log wall-clock timestamp cho `focus_lost`, `focus_active_detected`,
  `grace_complete`, và `first_send_after_focus`; ghi configured grace vào summary.
- **Protocol sau khi có instrumentation:**
  1. Test deterministic bằng fake clock/focus guard để chứng minh engine pause timeline và không burst
     event sau restore.
  2. Live probe các mức 0/25/50/100/150 ms theo thứ tự randomized, ít nhất 20 focus cycles/mức.
  3. Thu audio để xác nhận first post-focus note được game nhận; telemetry dùng để đo actual wall gap.
  4. Chọn một **global safety grace**, không profile-specific grace.
- **Quyết định tạm thời:** giữ giá trị bảo thủ hiện tại cho tới khi có instrumentation; không gọi
  50/100/150 ms là số đã kiểm nghiệm và không dùng O10.6 hiện tại để biện minh profile differentiation.
- **Mẫu báo cáo:**
  ```
  O10.6 focus grace | cycles/level: __ | randomized: yes/no
  Grace __ ms: accepted first notes __/__ | active->first-send p50/p95 __/__ ms
  Decision: global focus_restore_grace_us = __ / INCONCLUSIVE
  ```

---

## Part 3 — Folding results back

- `min_hold` là cần gạt timing production đã chứng minh. `repeat_release_gap` đã bị xoá khỏi
  profile/CLI/runtime policy; O10.4B chỉ còn là counterfactual audit. `input_lead`/`chord_merge`/
  `frame_align`/`release_gap` cũng đã bị xoá.
- `spin_threshold_us` và `focus_restore_grace_us` là hạ tầng toàn engine. O10.5/O10.6 phải dẫn tới
  global policy/safety value hoặc removal, không tạo thêm profile timing semantics.
- Với same-key gap mechanism, mechanism pass chưa đủ để reintroduce field: chỉ cân nhắc nếu O10.4B
  chứng minh positive schedule-changing binding trên corpus/tempo production và audio benefit rõ.
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

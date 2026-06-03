### T1 — Game samples input once per render frame; visibility floor = 1 frame

30fps 33-15/15 32-15/15 31-14/15(vẫn là 15 nhưng đôi khi mất nốt) 30-14/15 tức là bất đầu có dấu hiệu giảm

60fps 18-15/15(rõ ràng) 17-15/15 16-15/15 15-14/15 (đôi khi mất nốt) 14-14/15

144fps 7-15/15 6-(mất từ 3-4 nốt)

### T2 — Same-key repeat gap floor = max(~1.5 frame, ~18 ms)

| FPS | 2 bản pass 10/10 nhỏ nhất | Bắt đầu không ổn định | Nhận xét                                                                          |
| --: | ------------------------: | --------------------: | --------------------------------------------------------------------------------- |
|  60 |                 **24 ms** |       20 ms trở xuống | 16 ms có thể có run nhìn được, nhưng không repeatable giữa 2 bản                  |
| 144 |                 **16 ms** |       14 ms trở xuống | 144-02 có vẻ pass ở 14 ms, nhưng 144-01 miss vài onset, nên không coi là reliable |

| FPS |    Frame | 1.5 frame | Floor |  Gap dùng |
| --: | -------: | --------: | ----: | --------: |
|  60 | 16.67 ms |  25.00 ms | 18 ms | **25 ms** |
| 144 |  6.94 ms |  10.42 ms | 18 ms | **18 ms** |

với 1.5 frames + 18_000 µs floor thì nó hơi bảo thủ hơn mức quan sát được ở 144 FPS, nhưng đúng hướng vì 14 ms không ổn định và 60 FPS cần khoảng 24–25 ms để đạt 10/10 đáng tin cậy. ( nên xem xét lại chuẩn)

### T3 — Onset cadence is a fixed ~60 Hz tick (input lead must not scale with render FPS ≥60)

**EXP-1 (phía gửi sạch):**

| File      | FPS | SENT IOI mean |  SENT IOI std | IOI residual spread | lateness p95 | lateness max |
| --------- | --: | ------------: | ------------: | ------------------: | -----------: | -----------: |
| 60fps-01  |  60 |   119.9996 ms | **0.0364 ms** |            0.222 ms |     0.070 ms |     0.125 ms |
| 60fps-02  |  60 |   120.0003 ms | **0.0403 ms** |            0.253 ms |     0.080 ms |     0.163 ms |
| 144fps-01 | 144 |   120.0000 ms | **0.0525 ms** |            0.313 ms |     0.105 ms |     0.190 ms |
| 144fps-02 | 144 |   119.9999 ms | **0.0407 ms** |            0.249 ms |     0.085 ms |     0.189 ms |

Chênh lệch 60 → 144 chỉ cỡ vài microsecond đến vài chục microsecond, không phải dạng scale theo frame → vấn đề nhịp KHÔNG do player

**EXP-2 (jitter onset không giảm theo FPS):**

| File      | FPS | Matched | `[SENT] IOI std` | `[GAME] IOI std` | `[GAME-only residual] std` | Shift >5 ms |
| --------- | --: | ------: | ---------------: | ---------------: | -------------------------: | ----------: |
| 60fps-01  |  60 |   64/64 |         0.038 ms |        ~0.000 ms |               **0.024 ms** |        0/63 |
| 60fps-02  |  60 |   64/64 |         0.031 ms |      **7.96 ms** |                **5.35 ms** |        5/63 |
| 144fps-01 | 144 |   64/64 |         0.044 ms |     **10.06 ms** |                **8.25 ms** |       14/63 |
| 144fps-02 | 144 |   64/64 |         0.034 ms |        ~0.000 ms |               **0.023 ms** |        0/63 |

Dấu hiệu tick / beating

Bản quan trọng nhất là 144fps-01:

GAME IOI xuất hiện cụm: 181 ms / 219 ms, 180 ms / 221 ms
Residual nhảy: khoảng -19 ms đến -20 ms
Spread: ~21 ms residual, ~41 ms IOI

Đây là dấu hiệu rất giống quantization/bucket nội bộ: một onset bị kéo sớm khoảng 20 ms, onset kế tiếp bù lại thành khoảng 219–221 ms, nên tổng cadence dài hạn vẫn quanh 200 ms.

60fps-02 cũng có hiện tượng tương tự nhưng ít hơn: 5 event bị shift khoảng -19/-20 ms.

Hai bản còn lại (60fps-01, 144fps-02) gần như clean hoàn toàn, residual chỉ vài chục microsecond sau khi bỏ event đầu.

Mệnh đề này được hỗ trợ:Vấn đề không do player/sender.

Mệnh đề này được hỗ trợ một phần: Residuals có nhảy chu kỳ / bucket ~20 ms, nhất là @144.

Mệnh đề này chưa được chứng minh bởi bộ WAV này: [GAME-only jitter] std ≈13 ms @60 và ≈12 ms @144 gần như không đổi.

Dữ liệu thực tế không cho thấy jitter floor ổn định 12–13 ms. Nó cho thấy jitter kiểu phase/run-dependent: có run rất sạch, có run bị bucket-jump.

diễn đạt kết luận T3 chính xác hơn:

Onset jitter không đến từ sender. Một số capture cho thấy onset bị quantize/bucket nội bộ
khoảng ~20 ms, không cải thiện chỉ vì render FPS lên 144. Tuy nhiên jitter không xuất hiện
ổn định ở mọi run, nên đây có vẻ là phase-dependent internal game timing, không phải jitter floor luôn luôn ~12–13 ms.

### T4 — Wide audience floors were not shown necessary (remote)

Kết quả đo nhanh

| Test                           | Floor cao / `audience-safe` | Floor thấp / `local-precise` | Nhận xét                           |
| ------------------------------ | --------------------------: | ---------------------------: | ---------------------------------- |
| `metro_same_200`               |                    98 onset |                     95 onset | Gần ngang, không mất nốt hệ thống  |
| `repeat_staircase`             |                    71 onset |                     68 onset | Gần ngang, không có fail rõ        |
| `metro_same_200` valley drop   |                   ~-7.65 dB |                    ~-7.49 dB | Floor thấp **không kém hơn**       |
| `repeat_staircase` valley drop |                   ~-8.55 dB |                    ~-9.39 dB | Floor thấp còn tụt rõ hơn một chút |

Không thấy clipping:

| File                                 |         Peak |          RMS |
| ------------------------------------ | -----------: | -----------: |
| `metro-same-audience-safe.wav`       | ~-16.55 dBFS | ~-33.56 dBFS |
| `metro-same-local-precise.wav`       | ~-16.48 dBFS | ~-32.86 dBFS |
| `repeat-staircase-audience-safe.wav` | ~-13.73 dBFS | ~-33.34 dBFS |
| `repeat-staircase-local-precise.wav` | ~-12.62 dBFS | ~-32.98 dBFS |

Điểm quan trọng nhất: ở metro_same_200, valley drop của local-precise gần như bằng audience-safe, nên same-key repeat vẫn có down-up-down rõ. Không có dấu hiệu floor thấp làm nốt dính lại hoặc biến mất.

T4 — Wide audience floors were not shown necessary (remote)

Result:
Across remote captures, low-floor / frame-relative profiles passed close to the wide-floor audience_safe profile.

Observed:

- TEST_metro_same_200: low-floor profile preserved same-key repeats; valley drop was effectively equal to audience_safe.
- TEST_repeat_staircase: low-floor profile did not show systematic missing notes.
- Across multiple sessions, lower floors did not produce a repeatable failure pattern.

Conclusion:
Under the tested network conditions, wide absolute audience floors were not shown necessary.
Keep audience_safe as a conservative fallback, but it should no longer be treated as required by evidence.
Bad-network / high-jitter stress remains open and should be covered by O3/O7.

## Part 2 — Open calibration experiments

### O1 — `input_lead_us`: đo độ trễ thật (local và remote)

với local tiến hành dô lệch bằng cách kéo nốt đầu trùng với phách 0 với latency compensation -20 ms

--input-lead-ms 0 lệch sớm hơn 20ms
--input-lead-ms 20 lệch sớm hơn 20ms
--input-lead-ms 8 cũng lệch sớm 20ms
tất cả đều lệch sớm 20ms

với remote cũng có latency compensation -20 ms

--input-lead-ms 0 có nhiều giá trị nhưng giá trị sớm hơn xấp xỉ 40 ms tồn tại nhiều
--input-lead-ms 20
tôi nhận ra 1 điều đó là các note tiếp theo có thể đến sớm hoặc muộn hơn so với phách ở máy người dùng vì thế nên tôi nghĩ nó sẽ tự bù trừ, không nên thêm input lead ở đây, xử lý tốt ở local thì cũng sẽ xử lý tốt ở remote,

khả năng cao là input-lead-us đang không có tác dụng trong code, hoặc nếu có trong code base thì nó đang chưa có tác dụng theo ý nghĩa của nó

### O2 — `chord_merge_window_us`: ngưỡng làm bẹt

| `--chord-merge-window-ms` | Kết quả nghe/nhìn tín hiệu                                                 | Đánh giá |
| ------------------------: | -------------------------------------------------------------------------- | -------- |
|                       `0` | Giữ rõ roll: các note xuất hiện lệch nhau theo cụm ~18 ms                  | Safe     |
|                      `10` | Gần như giống `0`, vẫn giữ cảm giác rải                                    | Safe     |
|                      `20` | Bắt đầu merge/làm bẹt: không còn 4 onset rời rõ; có dấu hiệu gom thành cụm | Too high |
|                      `30` | Bẹt hơn nữa, cảm giác chord hơn arpeggio                                   | Too high |

nhưng 1 vấn đề đó là cách bài hát hiện tại không có các khoảng nhỏ như thế, 1 là chúng sẽ trùng thời gian nhau, và hai là cách nhau khoảng 100ms nên liệu chỉ số này có đang cần thiết hay không

### O3 — The real remote no-drop floor (sàn hold/gap audience theo số liệu)

với audience thì các note sẽ bị đến sớm hơn hoặc muộn hơn nên tôi nghĩ tạm thời nên làm chuẩn cho local-precise trước vì hiện tại cũng đã phát hiện được nhiều vấn đề liên quan đến timing ở local rồi, nếu xử lý tốt ở local thì cũng sẽ xử lý tốt ở remote, còn nếu xử lý tốt ở remote mà local vẫn có vấn đề thì cũng không ổn chút nào

\*\* Các vấn đề còn lại tạm gác lại để tập trung vào xử lý local trước

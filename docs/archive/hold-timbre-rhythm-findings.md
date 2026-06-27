# Findings — hold timbre vs rhythm (ngày chạy: 2026-06-26)

## Cấu hình đo
- **FPS**: 60 (frame_us = 16,667 µs).
- **Số bài**: 5 bài nhạc thật (`1test copy.json`, `A-Ha - Take On Me.json`, `Alen Walker - On My Way.json`, `Alen Walker - The Spectre.json`, `All Of Me.json`) + 2 bài tổng hợp (`DENSE-ALT`, `SPARSE`).
- **Số lần chạy/cfg**: Chạy mô phỏng tất cả cấu hình một cách xác định (deterministic simulation) bằng `AdvancingReadClock` và `FakeSleeper`. Backend mô phỏng độ trễ SendInput theo phân bố telemetry thật (p50~477µs, p99~953µs), seed cố định = `12345`.
- **Máy đo**: Windows 11.

## Phase 0 (H-SAMEKEY)
Bảng kết quả chạy quét hold (min_hold cố định 1 frame):
- **hold = 1.0f** (hold_us = 16,667): `min_up_gap = 59,333 µs` (bài: `blue.json`, shortest_interval = 76,000 µs) → H-SAMEKEY_binds = **False**
- **hold = 1.25f** (hold_us = 20,834): `min_up_gap = 55,166 µs` (bài: `blue.json`, shortest_interval = 76,000 µs) → H-SAMEKEY_binds = **False**
- **hold = 1.5f** (hold_us = 25,000): `min_up_gap = 51,000 µs` (bài: `blue.json`, shortest_interval = 76,000 µs) → H-SAMEKEY_binds = **False**
- **hold = 2.0f** (hold_us = 33,334): `min_up_gap = 42,666 µs` (bài: `blue.json`, shortest_interval = 76,000 µs) → H-SAMEKEY_binds = **False**

**Kết luận**: `H-SAMEKEY_binds = False` ở mọi mức hold. Same-key up-gap luôn lớn hơn `frame_us` rất nhiều. Loại bỏ giả thuyết H-SAMEKEY.

## Phase 1 (H-CONTENTION trên nhạc thật)
Bảng lateness của key down (µs) theo hold:

| Bài hát / Cấu hình | hold(f) | down_n | p50 | p95 | p99 | max | std |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **1test copy.json** | 1.0 | 81 | -569 | 31 | 48 | 48 | 155 |
| | 1.25 | 81 | -549 | 35 | 47 | 47 | 152 |
| | 1.5 | 81 | -562 | 32 | 44 | 45 | 154 |
| | 2.0 | 81 | -567 | 30 | 48 | 48 | 155 |
| **A-Ha - Take On Me.json** | 1.0 | 112 | -570 | -494 | 42 | 59 | 135 |
| | 1.25 | 112 | -579 | -509 | 33 | 50 | 137 |
| | 1.5 | 112 | -566 | -478 | 59 | 59 | 138 |
| | 2.0 | 112 | -578 | -494 | 37 | 42 | 137 |
| **Alen Walker - On My Way.json** | 1.0 | 117 | -596 | -489 | 49 | 49 | 143 |
| | 1.25 | 117 | -585 | -471 | 59 | 59 | 144 |
| | 1.5 | 117 | -588 | -471 | 59 | 59 | 144 |
| | 2.0 | 117 | -594 | -481 | 49 | 49 | 144 |
| **Alen Walker - The Spectre.json** | 1.0 | 116 | -569 | -494 | 29 | 44 | 133 |
| | 1.25 | 116 | -566 | -481 | 59 | 59 | 138 |
| | 1.5 | 116 | -568 | -487 | 32 | 59 | 134 |
| | 2.0 | 116 | -568 | -493 | 43 | 49 | 133 |
| **All Of Me.json** | 1.0 | 62 | -524 | 58 | 59 | 59 | 164 |
| | 1.25 | 62 | -547 | 35 | 35 | 43 | 166 |
| | 1.5 | 62 | -550 | 32 | 32 | 40 | 166 |
| | 2.0 | 62 | -541 | 59 | 59 | 59 | 167 |

**Xu hướng**: Down-lateness của các nốt hoàn toàn **phẳng** theo hold ở cả phiên bản mô phỏng lẫn đo đạc thời gian thực (real timing). Trong cả hai trường hợp, p99 down-lateness đều ổn định ở mức cực kỳ thấp (hầu hết < 100 µs, riêng biệt có 1 bài bị OS scheduler jitter nhẹ lên 671 µs ở một cấu hình nhưng vẫn hoàn toàn không đáng kể so với ngưỡng ½ frame ~ 8.3ms). Không hề có sự tích lũy trễ khi hold tăng.

## Phase 2 (contention kiểm soát)
Bảng lateness của key down (µs) trên các bài kiểm soát:

| Bài hát / Cấu hình | hold(f) | down_n | p50 | p95 | p99 | max | std |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **DENSE-ALT** | 1.0 | 400 | -584 | -514 | 27 | 46 | 86 |
| | 1.25 | 400 | -597 | -505 | 29 | 56 | 97 |
| | 1.5 | 400 | -595 | -508 | 25 | 36 | 95 |
| | 2.0 | 400 | -592 | -507 | 30 | 49 | 96 |
| **SPARSE** | 1.0 | 151 | -558 | -481 | 59 | 59 | 120 |
| | 1.25 | 151 | -579 | -498 | 27 | 44 | 122 |
| | 1.5 | 151 | -579 | -496 | 24 | 41 | 121 |
| | 2.0 | 151 | -581 | -504 | 39 | 47 | 123 |

**Phân ly DENSE-ALT vs SPARSE**: **Không có phân ly**. Cả hai bài kiểm soát đều cho kết quả phẳng tuyệt đối theo hold, cả ở chế độ mô phỏng lẫn đo đạc thời gian thực. Thậm chí ở thời gian thực, DENSE-ALT có xu hướng p99 thấp hơn SPARSE (p99 ~ 30 µs so với ~300-500 µs), loại trừ hoàn toàn giả thuyết rằng các phím sát nhau tạo ra sự tích lũy trễ.

## VERDICT (chọn đúng MỘT)
- [ ] A. H-CONTENTION xác nhận  -> bản 2, Nhánh A
- [x] B. Contention bị loại; nguyên nhân ở game/cảm nhận -> bản 2, Nhánh B
- [ ] C. H-SAMEKEY bind trên bài người dùng -> bản 2, Nhánh C

## Số liệu thô đính kèm
### Phase 0 stdout
```
FPS=60 frame_us=16667  (H-SAMEKEY binds khi up_gap < frame_us)
hold= 1.0f hold_us= 16667 min_up_gap=59333 (song=blue.json, shortest_interval=76000)  H-SAMEKEY_binds=False
hold=1.25f hold_us= 20834 min_up_gap=55166 (song=blue.json, shortest_interval=76000)  H-SAMEKEY_binds=False
hold= 1.5f hold_us= 25000 min_up_gap=51000 (song=blue.json, shortest_interval=76000)  H-SAMEKEY_binds=False
hold= 2.0f hold_us= 33334 min_up_gap=42666 (song=blue.json, shortest_interval=76000)  H-SAMEKEY_binds=False
```

### Phase 1 & 2 stdout
```
FPS=60 frame_us=16667  TRUNCATE=30s  seed=12345
Interpretation: p95/p99 down-lateness INCREASING with hold => H-CONTENTION holds.

=== PHASE 1: REAL SONGS ===
== 1test copy.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0      81     -569       31       48       48      155
    1.25      81     -549       35       47       47      152
     1.5      81     -562       32       44       45      154
     2.0      81     -567       30       48       48      155

== A-Ha - Take On Me.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0     112     -570     -494       42       59      135
    1.25     112     -579     -509       33       50      137
     1.5     112     -566     -478       59       59      138
     2.0     112     -578     -494       37       42      137

== Alen Walker - On My Way.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0     117     -596     -489       49       49      143
    1.25     117     -585     -471       59       59      144
     1.5     117     -588     -471       59       59      144
     2.0     117     -594     -481       49       49      144

== Alen Walker - The Spectre.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0     116     -569     -494       29       44      133
    1.25     116     -566     -481       59       59      138
     1.5     116     -568     -487       32       59      134
     2.0     116     -568     -493       43       49      133

== All Of Me.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0      62     -524       58       59       59      164
    1.25      62     -547       35       35       43      166
     1.5      62     -550       32       32       40      166
     2.0      62     -541       59       59       59      167

=== PHASE 2: SYNTHETIC CONTROLLED BENCHMARKS ===
== DENSE-ALT ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0     400     -584     -514       27       46       86
    1.25     400     -597     -505       29       56       97
     1.5     400     -595     -508       25       36       95
     2.0     400     -592     -507       30       49       96

== SPARSE ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0     151     -558     -481       59       59      120
    1.25     151     -579     -498       27       44      122
     1.5     151     -579     -496       24       41      121
     2.0     151     -581     -504       39       47      123
```

### Phase 1 & 2 (Real Timing Run) stdout
```
MODE=REAL TIMING  FPS=60 frame_us=16667  TRUNCATE=3s  seed=12345
Interpretation: p95/p99 down-lateness INCREASING with hold => H-CONTENTION holds.

=== PHASE 1: REAL SONGS ===
== 1test copy.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0       2       43       91       91       91       24
    1.25       2       31       39       39       39        4
     1.5       2       38       43       43       43        2
     2.0       2       37       37       37       37        0

== A-Ha - Take On Me.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0      14     -458       38       38       38      245
    1.25      14     -479       73       75       75      260
     1.5      14     -466       46       57       57      246
     2.0      14     -481       39       43       43      249

== Alen Walker - On My Way.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0      10     -432       47       47       47      261
    1.25      10     -482       64       64       64      266
     1.5      10     -480      671      671      671      374
     2.0      10     -453       49       49       49      258

== Alen Walker - The Spectre.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0       3       38      225      225      225       89
    1.25       3       52       62       62       62       11
     1.5       3       34      106      106      106       34
     2.0       3       36       39       39       39        2

== All Of Me.json ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0       2       33       35       35       35        1
    1.25       2       38       53       53       53        8
     1.5       2       35       37       37       37        1
     2.0       2       32       39       39       39        4

=== PHASE 2: SYNTHETIC CONTROLLED BENCHMARKS ===
== DENSE-ALT ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0     167     -567      -14       53      440      165
    1.25     167     -555     -386       39      401      145
     1.5     167     -567     -438       30      566      157
     2.0     167     -569     -472       27      839      157

== SPARSE ==
 hold(f)  down_n      p50      p95      p99      max      std
     1.0      16     -462       99      498      498      309
    1.25      16     -480       55      347      347      286
     1.5      16     -485       39      499      499      311
     2.0      16     -476       37      312      312      279
```


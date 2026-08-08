---
key: timing-engine
locale: vi
slug: timing-engine
title: Timing Engine
description: >-
  Cách Sky Auto Player lên lịch nốt nhạc, căn hợp âm theo dispatch frame, xử lý nốt hold,
  bù độ trễ máy và ý nghĩa thực sự của "độ chính xác timing" trong ngữ cảnh này.
summary: >-
  Timing engine của Sky Auto Player lên lịch nốt theo timeline căn theo frame, gửi hợp âm
  trong một lệnh SendInput duy nhất, giữ nốt hold đủ thời gian và đo độ trễ phía sender để
  giảm drift có hệ thống. Độ chính xác phía game phụ thuộc vào Windows và vòng lặp sampling của game.
category: playback-timing
order: 1
published: '2026-08-08'
updated: '2026-08-08'
draft: false
related:
  - how-it-works
  - security-boundaries
  - troubleshooting
evidence:
  - category: architecture
    label: Nguyên tắc timing
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/timing-principles.md
  - category: architecture
    label: Mô hình timing hold-frame
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/hold-frame-model.md
  - category: architecture
    label: Kiến trúc dispatch thời gian thực
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/rt-dispatch-architecture.md
  - category: implementation
    label: README — tóm tắt timing
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
---

## "Timing" có nghĩa gì ở đây

Sky Auto Player kiểm soát _phía sender_ — khi nào nó gọi `SendInput`. _Phía game_ — khi nào
game đăng ký và xử lý phím — phụ thuộc vào message queue Windows, tần số polling input của
game, và phần cứng của bạn. Đây là hai điều khác nhau.

Timing engine được thiết kế để tối thiểu hóa lỗi phía sender. Nó không — và không thể —
đảm bảo game nhận phím đúng lúc nhạc cần.

## Lên lịch nốt nhạc

Thay vì phát lại một chuỗi delay cố định, Sky Auto Player đặt từng nốt trên một timeline
được điều khiển bởi tempo của sheet và cài đặt FPS bạn cấu hình trong Sky. Khi scheduler
đến vị trí của một nốt trên timeline đó, nó dispatch phím.

Scheduler là **thuần túy**: không phụ thuộc wall-clock và không có lệnh gọi `SendInput`.
Nó tạo ra một lịch các sự kiện; các lớp infrastructure và platform thực thi các sự kiện đó.
Sự tách biệt này giúp logic timing có thể unit-test với đồng hồ được kiểm soát.

## Căn hợp âm

Các nốt thuộc cùng một hợp âm được gửi trong một lệnh `SendInput` duy nhất chứa tất cả
các phím trong hợp âm đó. Điều này loại bỏ độ lệch phía sender giữa các nốt phải vang lên
đồng thời. Game xử lý chúng đồng thời hay không tùy vào vòng lặp sampling của chính game.

## Ngữ nghĩa hold

Nốt hold (phím được giữ) được nhấn vào thời điểm bắt đầu nốt và nhả sau thời gian ký âm
đầy đủ. Nốt tiếp theo đến sát không khiến hold bị cắt ngắn — hold và nốt tiếp theo được
gửi như các sự kiện `SendInput` riêng biệt trên vị trí timeline của chúng.

## Chọn hold-frame

Thời gian hold được biểu diễn theo bội số của một frame game ở FPS đã cấu hình:

| Lựa chọn             | Thời gian ở 60 FPS |
| -------------------- | ------------------ |
| 1,0 frame (mặc định) | ~16,7 ms           |
| 1,25 frame           | ~20,8 ms           |
| 1,5 frame            | ~25,0 ms           |

Chọn 1,25 hoặc 1,5 nếu nốt hold ngắn không đăng ký ổn định trên máy của bạn. Cài đặt
FPS phải khớp với FPS cấu hình trong Sky để timing hold chính xác.

## Bù độ trễ

Sky Auto Player đo khoảng thời gian giữa khi lệnh gọi `SendInput` được lên lịch và khi
nó thực sự hoàn thành, xây dựng profile đang chạy về lỗi completion phía sender. Nó dùng
số đo này để điều chỉnh lead time của các dispatch tương lai — gửi sớm hơn một chút để
bù cho độ trễ quan sát được.

Đây là **bù phía sender mà thôi**. Sự thích ứng giảm drift có hệ thống do OS scheduling
jitter trên máy của bạn. Nó không điều chỉnh cho network latency, frame pacing của game,
hay bất kỳ yếu tố nào ở phía game. Telemetry hiển thị trong HUD báo cáo kết quả phía
sender đo được, không phải đảm bảo timing tuyệt đối.

## Rust dispatch worker

Một Rust worker sở hữu vòng lặp dispatch: biên dịch lịch nốt nhạc, timing wait độ phân
giải cao, theo dõi focus, lệnh gọi `SendInput`, dọn dẹp, và telemetry native. Python sở
hữu TUI và luồng ứng dụng. Chúng chạy trên các thread riêng biệt; build Python 3.14
free-threaded đảm bảo vòng lặp dispatch không tranh chấp với Textual UI trên GIL.

## Giới hạn đã biết

- Độ chính xác timing về phía game không do Sky Auto Player kiểm soát.
- Các đoạn nhạc rất nhanh (BPM cực cao hoặc arpeggio dày đặc) có thể vượt quá khả năng
  sampling rate của game, bất kể phím được gửi chính xác đến mức nào.
- MMCSS real-time scheduling yêu cầu Windows 10 hoặc 11 và không có sẵn trên các nền tảng khác.

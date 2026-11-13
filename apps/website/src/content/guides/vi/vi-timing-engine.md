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
  trong một lệnh SendInput duy nhất, áp dụng timing floor phía sender và dùng Frozen QPC waiter.
  Độ chính xác phía game phụ thuộc vào Windows và vòng lặp sampling của game.
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
    label: Kiến trúc playback engine
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
  - category: architecture
    label: Giao thức playback verification và qualification
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-verification.md
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

Thay vì phát lại một danh sách delay cố định, Sky Auto Player biên dịch các nốt thành một
luồng có thứ tự được điều khiển bởi tempo của sheet, FPS mục tiêu và các tham số hold.
Executor 15 slot quản lý nghĩa vụ hold phím ($H$) và release gap ($G$) một cách tất định,
không dùng hai con trỏ độc lập.

## Căn hợp âm

Các nốt thuộc cùng một hợp âm được gửi trong một lệnh `SendInput` duy nhất chứa tất cả
các phím trong hợp âm đó. Khi các sự kiện nhả phím và nhấn hợp âm trùng thời điểm, chúng
được gộp chung trong một syscall (Up trước Down) nhằm loại bỏ độ lệch phía sender.

## Ngữ nghĩa hold và timing floor

Thời điểm Up của nốt hold được kiểm soát bởi sàn hold phía sender $H$:
$$H = \lceil \text{hold\_frames} \times T \rceil + \text{margin}$$
với $T = \lceil 1{,}000{,}000 / \text{FPS} \rceil$. Việc nhấn lại cùng một phím phải tuân
thủ khoảng cách nhả phím $G = T$ trước khi phát Down tiếp theo.

## Chọn hold-frame

Thời gian hold được biểu diễn theo bội số của một frame game ở FPS đã cấu hình:

| Lựa chọn             | Thời gian ở 60 FPS |
| -------------------- | ------------------ |
| 1,0 frame (mặc định) | ~16,7 ms           |
| 1,25 frame           | ~20,8 ms           |
| 1,5 frame            | ~25,0 ms           |

FPS cấu hình dùng để tính floor phía sender. Nó không đảm bảo frame nào của game sẽ lấy mẫu
trạng thái phím.

## Hiệu chuẩn và robust wake guard

Sky Auto Player đo lường sai số thức giấc của Windows kernel timer trong quá trình khởi động
hiệu chuẩn host, xác định robust wake guard ($\text{median} + 6 \times \text{MAD}$). Trong quá
trình phát nhạc, production waiter sử dụng wake guard đóng băng (Frozen) này để kết hợp giấc ngủ
timer với vòng lặp spin QPC. Nó không điều chỉnh lead time liên tục để loại bỏ drift.

Đây là **timing phía sender mà thôi**. Nó giảm trễ hệ thống do OS scheduling jitter trên máy.
Nó không điều chỉnh cho network latency hay frame pacing của game. Telemetry hiển thị trong
bảng Diagnostics báo cáo kết quả thực tế phía sender, không phải đảm bảo âm thanh trong game.

## Rust dispatch worker

Một Rust worker thread chuyên trách sở hữu vòng lặp dispatch: biên dịch lịch nốt nhạc,
timing wait độ phân giải cao, theo dõi focus, lệnh gọi `SendInput`, dọn dẹp, và telemetry
native. Tauri/React shell render desktop UI, trong khi các Rust service native sở hữu
trạng thái playback và các chính sách an toàn.

## Giới hạn đã biết

- Độ chính xác timing về phía game không do Sky Auto Player kiểm soát.
- Các đoạn nhạc rất nhanh (BPM cực cao hoặc arpeggio dày đặc) có thể vượt quá khả năng
  sampling rate của game, bất kể phím được gửi chính xác đến mức nào.
- MMCSS real-time scheduling yêu cầu Windows 10 hoặc 11 và không có sẵn trên các nền tảng khác.

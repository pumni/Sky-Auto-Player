---
key: how-it-works
locale: vi
slug: how-it-works
title: Sky Auto Player Hoạt Động Như Thế Nào
description: >-
  Tìm hiểu cách Sky Auto Player chuẩn bị sheet, lên lịch nốt nhạc và gửi phím qua
  Windows SendInput API.
summary: >-
  Sky Auto Player chuẩn bị sheet do người dùng cung cấp và phát bằng player native trên
  Windows. Timing phía sender và việc game lấy mẫu input là hai việc riêng.
category: getting-started
order: 1
published: '2026-08-08'
updated: '2026-11-26'
draft: false
showDiagram: true
related:
  - sheet-formats
  - timing-engine
  - security-boundaries
evidence:
  - category: architecture
    label: Kiến trúc hệ thống
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/architecture.md
  - category: security
    label: Chính sách bảo mật
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: architecture
    label: Hợp đồng playback engine
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
---

## Phần mềm làm gì

Sky Auto Player đọc sheet do bạn cung cấp, chuẩn bị các nốt và hợp âm,
rồi lên lịch gửi phím trên desktop Windows. Game lấy mẫu input theo lịch
riêng, vì vậy timing phía sender không thể đảm bảo game đăng ký nốt ở frame nào.

Quy trình thông thường:

1. Export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/) dưới dạng JSON, **.skysheet**, hoặc TXT tương thích JSON.
2. Nhập sheet qua Library desktop.
3. Chọn bài, chuyển sang Sky và bắt đầu phát bằng điều khiển đã cấu hình.

Ứng dụng desktop được hỗ trợ là Tauri app dành cho Windows 11. Ứng dụng có
Library, Song Detail, Player Dock, Diagnostics, Settings và Update.

## Từ sheet đến playback

Desktop chuẩn bị và xác thực sheet trước khi bắt đầu phát. Sau đó một Rust
worker native gửi lịch đã chuẩn bị theo deadline QPC. Executor cố định 15 slot
theo dõi thời gian giữ phím và khoảng cách trước khi nhấn lại cùng phím. Một
hợp âm khỏe được gửi trong một prepared packet; nếu các lần nhả phím và nốt
mới đến hạn cùng lúc, sự kiện nhả được xếp trước lần nhấn mới.

Trước mỗi musical packet, native sender kiểm tra lại cửa sổ foreground hiện tại
và identity của target đã giữ. Foreground event chỉ là hint để kiểm tra lại focus,
không cấp quyền gửi. Safety cleanup có giới hạn có thể thử nhả riêng các phím
đã ghi trong journal. Tauri shell điều phối pause và stop, join worker rồi ghi
nhận retirement trước khi thông báo kết quả lên UI.

Xem hợp đồng playback engine tại
[tài liệu playback engine](https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md)
để biết quy tắc timing, cleanup và recovery hiện hành.

## Phạm vi và giới hạn

Sky Auto Player gửi phím Windows thông thường qua SendInput. Phần mềm không
kiểm soát vòng lấy mẫu input của game và không đảm bảo kết quả trong game.
[Chính sách bảo mật](https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md)
định nghĩa đầy đủ ranh giới bảo mật, bao gồm các kỹ thuật process và input bị cấm.

Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky. Sky Auto
Player là dự án cộng đồng không chính thức, không liên kết hay được
thatgamecompany chứng thực.

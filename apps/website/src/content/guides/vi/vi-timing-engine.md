---
key: timing-engine
locale: vi
slug: timing-engine
title: Timing Engine
description: >-
  Cách Sky Auto Player lên lịch nốt, áp dụng hold/release floor phía sender và vì sao timing
  phía game phụ thuộc vào Windows cùng vòng lấy mẫu của game.
summary: >-
  Executor cố định 15 slot lên lịch các nốt đã chuẩn bị và gửi hợp âm khỏe trong một packet.
  Timing phía sender và việc game lấy mẫu input là hai việc riêng.
category: playback-timing
order: 1
published: '2026-08-08'
updated: '2026-11-18'
draft: false
related:
  - how-it-works
  - security-boundaries
  - troubleshooting
evidence:
  - category: architecture
    label: Hợp đồng playback engine
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
  - category: test
    label: Playback verification
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-verification.md
---

## Timing có nghĩa gì

Sky Auto Player kiểm soát thời điểm gọi Windows SendInput. Lịch chạy của
Windows, vòng polling input của game và phần cứng ảnh hưởng thời điểm game
nhìn thấy phím. Sender hoàn tất không chứng minh game đã đăng ký nốt.

## Lên lịch nốt và hợp âm

Desktop chuẩn bị sheet trước khi khởi động native worker. Executor cố định
15 slot theo dõi các lần nhấn và nhả phím. Một hợp âm khỏe dùng một prepared
packet. Khi các lần nhả và nhấn độc lập cùng đến hạn, packet xếp tất cả Up
trước Down.

Production waiter dùng timing host đã hiệu chuẩn cùng deadline QPC. Wake guard
được giữ cố định trong lúc phát nhạc; hệ thống không điều chỉnh lead time liên
tục để che drift.

## Hold và release floor

Gọi T là ceiling của 1.000.000 chia cho FPS đã cấu hình, tính bằng
microsecond. Sender floor:

- H = ceiling(hold_frames × T) + margin.
- G = T + margin.

Up không được gửi trước thời điểm release trong sheet và thời điểm Down thành
công cộng H. Down cùng phím không được gửi trước khi Up thành công cộng G.
Đây là giới hạn phía sender; chúng không quyết định frame nào của game lấy
mẫu trạng thái phím.

## Giới hạn

Đoạn nhạc dày có thể vượt quá sampling rate của game dù sender đạt deadline.
Timing mô tả đường gửi input của Windows, không phải kết quả âm thanh hay
nghiệm thu gameplay. Desktop được hỗ trợ hướng đến Windows 11.

Xem [playback-engine.md](https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md)
và
[playback-verification.md](https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-verification.md)
để biết hợp đồng implementation và lệnh kiểm tra hiện hành.

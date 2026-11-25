---
key: troubleshooting
locale: vi
slug: troubleshooting
title: Xử Lý Sự Cố
description: >-
  Các vấn đề thường gặp với Sky Auto Player: bài hát không hiện, nốt lệch nhịp, hold không đăng ký,
  cảnh báo SmartScreen và lỗi cập nhật — kèm giải pháp.
summary: >-
  Xử lý sự cố theo triệu chứng cho Sky Auto Player: vấn đề Library, timing phát nhạc, đăng ký hold,
  SmartScreen và lỗi cập nhật. Bao gồm các giới hạn đã biết không thể sửa từ phía player.
category: support
order: 1
published: '2026-08-08'
updated: '2026-11-26'
draft: false
related:
  - windows-setup
  - sheet-formats
  - timing-engine
evidence:
  - category: implementation
    label: README — phần FAQ
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: architecture
    label: Kiến trúc playback engine
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
  - category: distribution
    label: Tài liệu phân phối và cập nhật
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/distribution-and-update.md
---

## Bài hát không hiện trong Library

**Nguyên nhân**: Phần mở rộng file không được nhận diện, hoặc sheet chưa được nhập vào playlist.

**Sửa**:

1. Xác nhận phần mở rộng là `.json`, `.skysheet`, hoặc `.txt`.
2. Tạo hoặc chọn playlist, rồi dùng **Add songs** để nhập file hoặc thư mục chứa file.
3. Kiểm tra file nguồn vẫn ở đường dẫn ban đầu; thao tác nhập tham chiếu file tại chỗ.

## Lỗi "Failed to parse" hoặc lỗi parse khi khởi động

**Nguyên nhân**: Nội dung file không phải JSON hợp lệ, hoặc sheet dùng schema version mà
parser hiện tại chưa hỗ trợ.

**Sửa**:

1. Mở file trong text editor. Xác nhận nó chứa JSON object bắt đầu bằng `{`.
2. Re-export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/).
3. Nếu lỗi vẫn tiếp diễn, mở [GitHub issue](https://github.com/pumni/Sky-Auto-Player/issues)
   và đính kèm thông báo lỗi (không phải file sheet).

## Nốt nghe muộn hoặc sớm so với nhạc nền

**Nguyên nhân A — Lệch FPS**: FPS trong profile của ứng dụng có thể không khớp cấu hình FPS trong Sky.

**Sửa**: Mở **Playback profile** trong Player Dock, vào **Advanced timing** và chọn FPS đã cấu hình trong Sky nếu biết. Các giá trị hỗ trợ là 30, 60, 90, 120, 144, 165 và 240. Timing phía sender không cho biết khi nào Sky lấy mẫu input.

**Nguyên nhân B — Lệch tempo**: Tempo đang chọn khác 1,0×.

**Sửa**: Trong **Playback profile** của Player Dock, đặt **Tempo** thành 1,0× để dùng tempo được ghi trong sheet mà không thêm hệ số tempo.

**Nguyên nhân C — Tải hệ thống cao**: Tải CPU cao có thể làm tăng jitter scheduling Windows và độ trễ dispatch phía sender. Wake guard đã hiệu chuẩn là cố định; nó không liên tục điều chỉnh lead theo thay đổi tải.

**Sửa**: Đóng ứng dụng nền và dùng **Diagnostics** để xem số đo dispatch phía sender. Playback profile có thể thay đổi tempo, hold và FPS đầu vào, nhưng không thể thay đổi lịch chạy Windows hay vòng lấy mẫu input của game. Xem [FAQ đầy đủ](https://pumni.github.io/Sky-Auto-Player/vi/faq/).

> **Lưu ý**: Ứng dụng kiểm soát thời điểm gọi SendInput. Khi game xử lý phím phụ thuộc vào vòng lấy mẫu input của game và lịch chạy Windows — các yếu tố nằm ngoài quyền kiểm soát của player.

## Nốt hold không đăng ký (nhả quá sớm)

**Nguyên nhân**: Hold floor phía sender không bảo đảm game lấy mẫu và đăng ký phím theo thời điểm đó.

**Sửa**: Trong **Playback profile** của Player Dock, thử đặt **Base Hold** thành 1,25 hoặc 1,5 frame. Hold floor và timing margin ảnh hưởng thời điểm đủ điều kiện nhả phía sender; chúng không bảo đảm khi nào Sky đăng ký phím.

## Phát nhạc dừng hoặc nốt không được đăng ký

**Nguyên nhân — Foreground target**: Mỗi musical packet mới đều cần kiểm tra lại cửa sổ foreground hiện tại và identity của target đã giữ. Foreground event chỉ là hint để kiểm tra lại, không thể cấp quyền gửi. Khi mất authority, engine ngăn các musical packet mới.

**Sửa**: Đưa target Sky đã chọn lên foreground trước khi bắt đầu hoặc tiếp tục phát. Nếu ứng dụng dừng do mất focus hay target, khôi phục target rồi kiểm tra trạng thái phát trong Player Dock trước khi bắt đầu lại. Safety cleanup có giới hạn có thể thử nhả riêng các phím đã được ghi trong release journal. Nếu phím đã gửi nhưng game không đăng ký, hãy xem lại giới hạn timing phía sender ở trên và trong [hướng dẫn timing](https://pumni.github.io/Sky-Auto-Player/vi/guides/timing-engine/).

## Cảnh báo Windows SmartScreen khi khởi động

**Nguyên nhân**: Installer canonical hiện chưa được ký Authenticode theo chính sách hiện hành. Windows có thể hiển thị Unknown Publisher hoặc cảnh báo SmartScreen.

**Sửa**: Xác nhận file được tải từ [trang GitHub Releases chính thức của Sky Auto Player](https://github.com/pumni/Sky-Auto-Player/releases). Tauri updater xác minh sidecar `.exe.sig` riêng; sidecar không cung cấp publisher identity Authenticode. Nếu không xác nhận được nguồn tải, đừng chạy installer.

## Cập nhật thủ công

**Nguyên nhân**: Official Tauri updater không thể stage hoặc chạy installer current-user.

**Sửa**:

1. Thử lại **Update and Restart** từ thông báo cập nhật trong app.
2. Nếu vẫn thất bại, tải installer NSIS canonical từ [trang GitHub Releases chính thức của Sky Auto Player](https://github.com/pumni/Sky-Auto-Player/releases).
3. Không dùng ZIP v3, `MANIFEST.json` hoặc updater executable độc lập cho v4.

## HUD hiển thị jitter timing cao

**Nguyên nhân**: Chỉ số jitter của HUD phản ánh phương sai dispatch _phía sender_. Giá trị cao
cho thấy áp lực scheduling OS trên máy của bạn.

**Sửa**: Đóng ứng dụng ngốn tài nguyên và xem lại các số đo phía gửi. Những số đo này không cho biết khi nào Sky đăng ký đầu vào hay phát ra âm thanh.

## Giới hạn đã biết

Những vấn đề này không thể sửa từ phía player và được ghi nhận là ràng buộc đã biết:

- **Timing phía game**: Game quyết định khi nào đăng ký phím. Sky Auto Player không thể
  ghi đè tần số polling input của game.
- **Đoạn nhạc rất nhanh**: Arpeggio cực dày ở BPM cao có thể vượt quá khả năng đăng ký
  nốt riêng lẻ của game, bất kể phím được gửi chính xác đến mức nào.
- **Windows 11 64-bit**: Bản đóng gói hiện tại cần Windows 11 64-bit. Không hỗ trợ gói
  Windows 10, macOS hay Linux.
- **Dự án không chính thức**: Sky Auto Player không liên kết với thatgamecompany. Tự động
  phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.

---
key: troubleshooting
locale: vi
slug: troubleshooting
title: Xử Lý Sự Cố
description: >-
  Các vấn đề thường gặp với Sky Auto Player: bài hát không hiện, nốt lệch nhịp, hold không đăng ký,
  cảnh báo SmartScreen và lỗi cập nhật — kèm giải pháp.
summary: >-
  Xử lý sự cố theo triệu chứng cho Sky Auto Player: vấn đề picker, timing phát nhạc, đăng ký hold,
  SmartScreen và lỗi cập nhật. Bao gồm các giới hạn đã biết không thể sửa từ phía player.
category: support
order: 1
published: '2026-08-08'
updated: '2026-08-08'
lastReviewedVersion: '3.0.0'
draft: false
evidence:
  - category: implementation
    label: README — phần FAQ
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: architecture
    label: Nguyên tắc timing
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/timing-principles.md
  - category: distribution
    label: Tài liệu phân phối và cập nhật
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/distribution-and-update.md
---

## Bài hát không hiện trong picker

**Nguyên nhân**: Phần mở rộng file không được nhận diện, hoặc picker chưa được reload.

**Sửa**:

1. Xác nhận phần mở rộng là `.json`, `.skysheet`, hoặc `.txt`.
2. Nhấn `Ctrl+R` trong picker để reload mà không cần khởi động lại.
3. Nếu vẫn không hiện, kiểm tra file nằm trực tiếp trong thư mục `songs/`, không phải trong thư mục con.

## Lỗi "Failed to parse" hoặc lỗi parse khi khởi động

**Nguyên nhân**: Nội dung file không phải JSON hợp lệ, hoặc sheet dùng schema version mà
parser hiện tại chưa hỗ trợ.

**Sửa**:

1. Mở file trong text editor. Xác nhận nó chứa JSON object bắt đầu bằng `{`.
2. Re-export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/).
3. Nếu lỗi vẫn tiếp diễn, mở [GitHub issue](https://github.com/pumni/Sky-Auto-Player/issues)
   và đính kèm thông báo lỗi (không phải file sheet).

## Nốt nhạc nghe muộn hoặc sớm so với nhạc nền

**Nguyên nhân A — Lệch FPS**: Cài đặt FPS trong Sky Auto Player không khớp với FPS cấu hình trong Sky.

**Sửa**: Mở cài đặt theo bài và đặt FPS khớp với frame rate của Sky (30 hoặc 60).

**Nguyên nhân B — Lệch tempo**: Tempo multiplier không phải 1,0.

**Sửa**: Reset tempo multiplier về 1,0 trong cài đặt theo bài.

**Nguyên nhân C — Tải hệ thống cao**: CPU tải nặng gây jitter scheduling Windows, làm tăng
độ trễ phía sender vượt quá khả năng bù của adaptive lead.

**Sửa**: Đóng ứng dụng nền. Thử bật tuning preset cho PC yếu qua flag `--tuning`. Xem
[FAQ đầy đủ](https://pumni.github.io/Sky-Auto-Player/vi/faq/).

> **Lưu ý**: Sky Auto Player kiểm soát khi nào phím được _gửi_. Khi game _xử lý_ chúng
> phụ thuộc vào vòng lặp sampling input của game và scheduling Windows — những yếu tố này
> ngoài tầm kiểm soát của player.

## Nốt hold không đăng ký (nhả quá sớm)

**Nguyên nhân**: Thời gian hold ở FPS đã cấu hình ngắn hơn thời gian đăng ký tối thiểu
của game cho phím đó.

**Sửa**: Trong cài đặt theo bài, chuyển từ **1,0 frame** sang **1,25 frame** hoặc
**1,5 frame**. Cài đặt FPS phải khớp với FPS trong Sky để timing hold chính xác.

## Game không nhận input nào

**Nguyên nhân A — Focus**: Cửa sổ game không có focus khi phát nhạc bắt đầu.

**Sửa**: Chuyển sang cửa sổ game trước khi nhấn hotkey phát. Sky Auto Player gửi phím đến
cửa sổ đang active; nếu game không được focus, phím sẽ đi nơi khác.

**Nguyên nhân B — Hotkey chưa đặt**: Hotkey phát chưa được bind hoặc xung đột với ứng dụng khác.

**Sửa**: Mở command palette (`/`) và kiểm tra hotkey binding.

**Nguyên nhân C — Sai key layout**: Profile key layout không khớp với keyboard layout đang
dùng trong Sky.

**Sửa**: Kiểm tra profile layout trong cài đặt khớp với những gì Sky hiển thị.

## Cảnh báo Windows SmartScreen khi khởi động

**Nguyên nhân**: Sky Auto Player chưa được ký mã. SmartScreen hiển thị cảnh báo này
cho bất kỳ tệp unsigned nào tải từ internet.

**Sửa**: Nhấn **Thêm thông tin → Vẫn chạy**. Để xác minh binary hợp lệ, kiểm tra
checksum SHA256 của ZIP tải về so với file `.sha256` được công bố trên
[trang releases](https://github.com/pumni/Sky-Auto-Player/releases/latest).

## Cập nhật thất bại hoặc rollback

**Nguyên nhân**: Checksum SHA256 của archive tải về không khớp với giá trị công bố, hoặc
tải xuống bị gián đoạn.

**Sửa**:

1. Kiểm tra kết nối internet.
2. Chạy lại `updater.bat -Channel stable` — updater sẽ thử lại tải về.
3. Nếu updater vẫn thất bại, tải ZIP thủ công từ trang releases và giải nén cạnh cài đặt
   hiện tại (thư mục `config.json` và `songs/` sẽ không bị updater ghi đè).

## HUD hiển thị jitter timing cao

**Nguyên nhân**: Chỉ số jitter của HUD phản ánh phương sai dispatch _phía sender_. Giá trị cao
cho thấy áp lực scheduling OS trên máy của bạn.

**Sửa**: Đóng ứng dụng nền. Thử tuning preset. Lưu ý rằng jitter phía sender không trực
tiếp dẫn đến lỗi timing nghe được — sampling rate của game thường là yếu tố lớn hơn.

## Giới hạn đã biết

Những vấn đề này không thể sửa từ phía player và được ghi nhận là ràng buộc đã biết:

- **Timing phía game**: Game quyết định khi nào đăng ký phím. Sky Auto Player không thể
  ghi đè tần số polling input của game.
- **Đoạn nhạc rất nhanh**: Arpeggio cực dày ở BPM cao có thể vượt quá khả năng đăng ký
  nốt riêng lẻ của game, bất kể phím được gửi chính xác đến mức nào.
- **Chỉ hỗ trợ Windows**: MMCSS real-time scheduling và `SendInput` là Windows API. macOS
  và Linux không được hỗ trợ và không có trong roadmap.
- **Dự án không chính thức**: Sky Auto Player không liên kết với thatgamecompany. Tự động
  phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.

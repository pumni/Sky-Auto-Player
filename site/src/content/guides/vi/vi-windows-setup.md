---
key: windows-setup
locale: vi
slug: windows-setup
title: Cài Đặt Windows và Lần Khởi Động Đầu Tiên
description: >-
  Hướng dẫn từng bước cài đặt Sky Auto Player trên Windows 10 hoặc 11: tải xuống, giải nén,
  xác minh checksum, thêm bài hát và cấu hình hotkey.
summary: >-
  Tải ZIP từ GitHub releases, giải nén vào bất kỳ đâu — không cần cài đặt, không cần quyền admin.
  Thả file bài vào songs/, chạy Sky-Auto-Player.exe, dùng hotkey điều khiển phát nhạc.
category: getting-started
order: 3
published: '2026-08-08'
updated: '2026-08-08'
lastReviewedVersion: '3.0.0'
draft: false
related:
  - sheet-formats
  - troubleshooting
  - security-boundaries
evidence:
  - category: implementation
    label: README — quick start và yêu cầu hệ thống
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: distribution
    label: Tài liệu phân phối và cập nhật
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/distribution-and-update.md
---

## Yêu cầu hệ thống

- Windows 10 hoặc 11, 64-bit
- Không cần Python hệ thống — bản đóng gói đi kèm runtime riêng
- Không cần quyền quản trị viên để sử dụng bình thường
- Không tạo registry entry, không cài đặt toàn hệ thống

## Tải xuống và giải nén

1. Vào trang [release mới nhất](https://github.com/pumni/Sky-Auto-Player/releases/latest) trên GitHub.
2. Tải `Sky-Auto-Player-v<phiên bản>.zip`.
3. _(Tùy chọn)_ Tải `Sky-Auto-Player-v<phiên bản>.zip.sha256` và xác minh checksum trước khi giải nén:

   ```powershell
   # Trong PowerShell, ở thư mục chứa file zip:
   (Get-FileHash "Sky-Auto-Player-v3.0.0.zip" -Algorithm SHA256).Hash
   # So sánh kết quả với nội dung file .sha256
   ```

4. Giải nén ZIP vào bất kỳ thư mục nào, ví dụ `C:\Sky-Auto-Player\`.
5. Chạy `Sky-Auto-Player.exe`.

## Lần khởi động đầu tiên

Lần đầu khởi động, ứng dụng tạo file `config.json` trong thư mục cài đặt. Bạn không cần
chỉnh sửa file này thủ công — tất cả cài đặt có thể truy cập từ bên trong TUI.

Nếu Windows hiện cảnh báo SmartScreen ("Windows đã bảo vệ máy tính của bạn"), nhấn
**Thêm thông tin → Vẫn chạy**. Tệp thực thi chưa được ký mã; SmartScreen hiển thị cảnh
báo này cho bất kỳ tệp unsigned nào tải từ internet.

## Thêm bài hát

1. Export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/) dưới dạng JSON, `.skysheet`, hoặc TXT.
2. Sao chép hoặc di chuyển file vào thư mục `songs/` cạnh `Sky-Auto-Player.exe`.
3. Trong picker, nhấn `Ctrl+R` để reload danh sách.

## Hotkey

| Hotkey      | Hành động                |
| ----------- | ------------------------ |
| `/`         | Mở command palette       |
| `F8`        | Tạm dừng / tiếp tục phát |
| `F9`        | Bỏ qua phần tiếp theo    |
| `F10`       | Dừng phát                |
| `q` / `Esc` | Thoát                    |
| `Ctrl+R`    | Reload danh sách bài     |

## Cập nhật

Sky Auto Player kiểm tra GitHub để tìm bản phát hành mới khi khởi động và hiển thị banner
khi có bản mới. Để cập nhật:

1. Đóng Sky Auto Player.
2. Chạy `updater.bat` trong thư mục cài đặt.
3. Mở lại `Sky-Auto-Player.exe`.

Updater xác minh checksum SHA256 của archive tải về trước khi chạm vào bất kỳ file nào.
Nếu xác minh thất bại, nó rollback và giữ nguyên cài đặt của bạn. Không bao giờ thay thế
`config.json` hay thư mục `songs/`.

## Gỡ cài đặt

Xóa thư mục cài đặt. Sky Auto Player không ghi vào registry hay bất kỳ vị trí nào ngoài
thư mục của nó.

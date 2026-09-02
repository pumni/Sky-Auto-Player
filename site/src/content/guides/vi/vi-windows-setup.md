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
- Không cần Python hệ thống — bản đóng gói là native và độc lập
- Không cần quyền quản trị viên để sử dụng bình thường
- Không tạo registry entry, không cài đặt toàn hệ thống

## Tải xuống và giải nén

1. Vào trang [release mới nhất](https://github.com/pumni/Sky-Auto-Player/releases/latest) trên GitHub.
2. Tải `Sky-Auto-Player-v<phiên bản>.zip`.
3. _(Tùy chọn)_ Tải `Sky-Auto-Player-v<phiên bản>.zip.sha256` và xác minh checksum trước khi giải nén:

   ```powershell
   # Trong PowerShell, ở thư mục chứa file zip:
   (Get-FileHash "Sky-Auto-Player-v<version>.zip" -Algorithm SHA256).Hash
   # So sánh kết quả với nội dung file .sha256
   ```

4. Giải nén ZIP vào bất kỳ thư mục nào, ví dụ `C:\Sky-Auto-Player\`.
5. Chạy `Sky-Auto-Player.exe`.

## Lần khởi động đầu tiên

Lần đầu khởi động, ứng dụng tạo file `config.json` trong thư mục cài đặt. Bạn không cần
chỉnh sửa file này thủ công — mọi cài đặt được hỗ trợ đều có trong hộp thoại **Settings** của
ứng dụng desktop.

Nếu Windows hiện cảnh báo SmartScreen ("Windows đã bảo vệ máy tính của bạn"), nhấn
**Thêm thông tin → Vẫn chạy**. Tệp thực thi chưa được ký mã; SmartScreen hiển thị cảnh
báo này cho bất kỳ tệp unsigned nào tải từ internet.

## Thêm bài hát

1. Export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/) dưới dạng JSON, `.skysheet`, hoặc TXT.
2. Sao chép hoặc di chuyển file vào thư mục `songs/` cạnh `Sky-Auto-Player.exe`.
3. Trong Library desktop, nhấn **Reload songs**.

## Phím tắt desktop

| Phím tắt | Hành động                    |
| -------- | ---------------------------- |
| `/`      | Focus ô tìm kiếm bài         |
| `Ctrl+F` | Focus ô tìm kiếm bài         |
| `Esc`    | Đóng overlay an toàn đang mở |
| `q`      | Không thoát desktop GUI      |

## Cập nhật

Sky Auto Player kiểm tra GitHub để tìm bản phát hành mới khi khởi động và hiển thị banner
khi có bản mới. Chọn **Open GitHub Releases** để tải bản cập nhật thủ công:

1. Tải ZIP tương ứng, `.zip.sha256`, `MANIFEST.json` và `MANIFEST.json.sig` từ
   [trang GitHub Releases chính thức](https://github.com/pumni/Sky-Auto-Player/releases).
2. Khi cần, xác minh SHA256 của ZIP và manifest chính xác.
3. Giải nén ZIP vào thư mục mới rồi chép `config.json`, `.env`, `songs/` và `logs/` của bạn
   vào đó.
4. Chạy `Sky-Auto-Player.exe` từ thư mục mới.

Binary Windows công khai được cố ý để unsigned và không có system installer. Dùng thao tác
**Update and Restart** trong app để cập nhật bằng native updater đã xác minh; nếu không thể
stage, hãy tải gói canonical thủ công như trên. Authenticode là **N/A — intentionally
unsigned**; Windows SmartScreen có thể hiện cảnh báo ứng dụng không được nhận diện.

## Gỡ cài đặt

Xóa thư mục cài đặt. Sky Auto Player không ghi vào registry hay bất kỳ vị trí nào ngoài
thư mục của nó.

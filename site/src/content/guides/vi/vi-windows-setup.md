---
key: windows-setup
locale: vi
slug: windows-setup
title: Cài Đặt Windows và Lần Khởi Động Đầu Tiên
description: >-
  Hướng dẫn từng bước cài đặt Sky Auto Player trên Windows 10 hoặc 11: tải installer Tauri,
  cài đặt, thêm bài hát và cấu hình hotkey.
summary: >-
  Tải installer Tauri NSIS canonical, cài cho current user và nhập sheet qua Library desktop.
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
- Không cần quyền quản trị viên cho cài đặt current-user
- Không có installer toàn hệ thống hoặc service

## Tải xuống và cài đặt

1. Vào [release authority v4](https://github.com/pumni/Sky-Auto-Player-Releases/releases).
2. Tải installer Tauri NSIS canonical và sidecar `.exe.sig`.
3. Chạy installer và giữ vị trí current-user mặc định.
4. Mở **Sky Auto Player** từ shortcut đã cài.

## Lần khởi động đầu tiên

Lần đầu khởi động, ứng dụng tạo dữ liệu trong vùng app-data của user. Bạn không cần chỉnh sửa
thủ công — mọi cài đặt được hỗ trợ đều có trong hộp thoại **Settings**.

Installer canonical được xác minh Authenticode. Nếu Windows báo lỗi integrity hoặc publisher,
hãy xóa file và tải lại từ release authority v4.

## Thêm bài hát

1. Export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/) dưới dạng JSON, `.skysheet`, hoặc TXT.
2. Nhập file qua Library desktop.
3. Dùng nút refresh của Library nếu file được thêm từ trình chọn file bên ngoài.

## Phím tắt desktop

| Phím tắt | Hành động                    |
| -------- | ---------------------------- |
| `/`      | Focus ô tìm kiếm bài         |
| `Ctrl+F` | Focus ô tìm kiếm bài         |
| `Esc`    | Đóng overlay an toàn đang mở |
| `q`      | Không thoát desktop GUI      |

## Cập nhật

Sky Auto Player kiểm tra channel v4 qua release authority riêng và hiển thị banner khi có bản
mới. **Update and Restart** dùng official Tauri updater qua `UpdateService` Rust; updater xác
minh `.sig` rồi chạy installer current-user. V4 không có `Sky-Auto-Player-Updater.exe`, updater
ZIP portable hoặc hợp đồng `MANIFEST.json.sig` tùy biến.

## Gỡ cài đặt

Dùng **Installed apps** của Windows để gỡ gói current-user. Dữ liệu app trong profile user là
một vùng riêng và có thể xóa sau khi sao lưu dữ liệu cần giữ.

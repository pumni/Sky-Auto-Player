---
key: windows-setup
locale: vi
slug: windows-setup
title: Cài Đặt Windows và Lần Khởi Động Đầu Tiên
description: >-
  Hướng dẫn cài đặt Sky Auto Player trên Windows 11, nhập sheet và tìm playback profile cùng các điều khiển desktop.
summary: >-
  Tải installer Tauri NSIS canonical, cài cho current user và nhập sheet qua Library desktop.
category: getting-started
order: 3
published: '2026-08-08'
updated: '2026-11-26'
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

- Windows 11, 64-bit
- Không cần developer toolchain hay scripting runtime riêng — bản đóng gói chạy độc lập
- Không cần quyền quản trị viên cho cài đặt current-user
- Không có installer toàn hệ thống hoặc service

## Tải xuống và cài đặt

1. Vào [trang GitHub Releases chính thức của Sky Auto Player](https://github.com/pumni/Sky-Auto-Player/releases/latest).
2. Tải installer Tauri NSIS canonical. Sidecar `.exe.sig` dành cho in-app updater, không cần khi cài thủ công.
3. Chạy installer và giữ vị trí current-user mặc định.
4. Mở **Sky Auto Player** từ shortcut đã cài.

## Lần khởi động đầu tiên

Lần đầu khởi động, ứng dụng tạo dữ liệu trong vùng app-data của user. Bạn không cần chỉnh sửa thủ công. **Settings** quản lý các tùy chọn chung của ứng dụng, bao gồm playback defaults. **Playback profile** trong Player Dock giúp truy cập nhanh **Base Hold**, **Tempo** và **Advanced timing** (FPS và timing margin).

Installer canonical hiện chưa được ký Authenticode theo chính sách đang áp dụng. Windows có thể hiển thị Unknown Publisher hoặc cảnh báo SmartScreen. Chỉ tải từ trang GitHub Releases chính thức. Tauri updater xác minh sidecar .exe.sig riêng; sidecar này không cần cho cài đặt thủ công.

## Thêm bài hát

1. Export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/) dưới dạng JSON, `.skysheet`, hoặc TXT.
2. Tạo hoặc chọn playlist, rồi dùng **Add songs** để nhập file hoặc thư mục chứa file.
3. File được nhập vẫn ở vị trí gốc; ứng dụng không di chuyển hay xóa chúng.

Để phát nhạc, hãy mở nhạc cụ 15 phím tương thích trong Sky. Chọn bài trong Sky Auto Player rồi nhấn **Play**. Giữ tiêu điểm ở cửa sổ Sky khi phát.

## Phím tắt desktop

| Phím tắt | Hành động            |
| -------- | -------------------- |
| /        | Focus ô tìm kiếm bài |
| Ctrl+F   | Focus ô tìm kiếm bài |
| Ctrl+,   | Mở Settings          |
| Ctrl+I   | Bật/tắt Inspector    |
| Esc      | Đóng overlay đang mở |

## Cập nhật

Sky Auto Player kiểm tra channel v4 qua metadata đã được xác thực trên branch `release-metadata`
trong repository chính và hiển thị banner khi có bản
mới. **Update and Restart** dùng official Tauri updater qua `UpdateService` Rust; updater xác
minh `.sig` rồi chạy installer current-user. V4 không có `Sky-Auto-Player-Updater.exe`, updater
ZIP portable hoặc hợp đồng `MANIFEST.json.sig` tùy biến.

## Gỡ cài đặt

Dùng **Installed apps** của Windows để gỡ gói current-user. Dữ liệu app trong profile user là
một vùng riêng và có thể xóa sau khi sao lưu dữ liệu cần giữ.

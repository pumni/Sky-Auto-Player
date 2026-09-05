---
key: security-boundaries
locale: vi
slug: security-boundaries
title: Ranh Giới Bảo Mật
description: >-
  Ba mandate bảo mật không thể thương lượng của Sky Auto Player: chỉ SendInput, không can
  thiệp game, xác thực input nghiêm ngặt. Mã nguồn công khai và được kiểm tra tự động trên CI.
summary: >-
  Sky Auto Player chỉ dùng Windows SendInput API cho phím bấm. Không đọc bộ nhớ game, không
  sửa file game, không hook process, không inject code. Ba mandate bảo mật được thực thi trên
  CI ở mỗi commit.
category: technical-safety
order: 1
published: '2026-08-08'
updated: '2026-08-08'
draft: false
related:
  - how-it-works
  - windows-setup
  - timing-engine
evidence:
  - category: security
    label: SECURITY.md — chính sách bảo mật đầy đủ
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: security
    label: Script kiểm tra bảo mật (CI gate)
    url: https://github.com/pumni/Sky-Auto-Player/tree/main/rust/xtask
  - category: implementation
    label: Lớp platform Windows — nơi duy nhất SendInput được phép tồn tại
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/rust/crates/sky_dispatch_win32/src/
  - category: distribution
    label: Hợp đồng phân phối — Tauri NSIS và updater chính thức
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/distribution-and-update.md
---

## Ba mandate bảo mật

Sky Auto Player được xây dựng trên ba quy tắc không thể thương lượng, được thực thi trên CI
ở mỗi lần push và pull request. Vi phạm bất kỳ quy tắc nào đều gây ra build failure ngay lập tức.

### 1. Không can thiệp game

Sky Auto Player **không bao giờ**:

- Đọc hay ghi bộ nhớ của bất kỳ process nào khác
- Sửa đổi, vá lỗi, hay kiểm tra file game
- Inject DLL vào bất kỳ process nào
- Attach debugger vào bất kỳ process nào
- Cài đặt Windows hook (`SetWindowsHookEx`, `SetWinEventHook`, v.v.) cho bất kỳ mục tiêu nào
- Bypass hệ thống chống gian lận

Các hạn chế này áp dụng cho tất cả code trong repository — không chỉ code nhắm mục tiêu vào game.

### 2. Chỉ dùng SendInput

Cơ chế duy nhất để mô phỏng input bàn phím là `user32.SendInput`. Các lệnh gọi cũ
`keybd_event` / `mouse_event` và bất kỳ thư viện input bên thứ ba nào (`python-keyboard`,
`pynput`, v.v.) đều bị cấm. Lớp platform Windows (`rust/crates/sky_dispatch_win32/`) là nơi duy nhất
trong codebase nơi `SendInput` và Win32 types được phép tồn tại.

### 3. Xác thực input nghiêm ngặt

Mọi CLI argument, trường config, file bài hát, hotkey binding và tham số timing đều được
xác thực qua một cấu trúc dữ liệu có type trước khi đến dispatch engine. Input không hợp lệ
bị từ chối với thông báo lỗi rõ ràng — không bị coerce im lặng.

## Thực thi trên CI

Rust audit trong `cargo xtask check static` quét native product ở mỗi lần push và pull request.
Bất kỳ commit nào đưa vào lệnh gọi API bị cấm (hook, đọc bộ nhớ, remote thread, debug attach)
đều fail CI ngay. Các ngoại lệ lịch sử, nếu có, được liệt kê trong `.config/security_audit_baseline.json`
với lý do giải thích và tham chiếu tracking.

Để chạy kiểm tra cục bộ:

```powershell
cargo xtask check static
```

## Mã nguồn

Sky Auto Player là mã nguồn mở theo GNU General Public License v3.0. Toàn bộ mã nguồn
có tại [github.com/pumni/Sky-Auto-Player](https://github.com/pumni/Sky-Auto-Player). Bạn
có thể kiểm tra code, build từ source, hoặc xác minh binary release khớp với source.

## Xác minh bản phát hành

V4 phát hành installer Tauri NSIS canonical và sidecar `.exe.sig`. Installer được xác minh
Authenticode; qualification ràng buộc đúng byte của installer và signature bằng SHA-256 cùng
evidence SPDX SBOM và provenance.

## Ranh giới cập nhật công khai

`UpdateService` Rust gọi official Tauri updater. Tauri xác minh `.exe.sig` trước khi chạy
installer current-user. V4 không có custom updater executable đi kèm, updater ZIP portable,
hay hợp đồng `MANIFEST.json.sig`; authority của v4 là repository release riêng, còn endpoint
channel và downgrade policy không đi qua frontend.

## Thông báo Điều khoản Dịch vụ

Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky: Children of the Light.
Sky Auto Player là dự án cộng đồng không chính thức và không có liên kết hay sự chứng thực
từ thatgamecompany. Dùng có trách nhiệm và tự chịu rủi ro.

## Báo cáo lỗ hổng

Gửi email đến **pumni.dev@gmail.com**. Không mở issue công khai với các bước tái tạo.
Mong đợi xác nhận trong vòng 7 ngày và quyết định triage trong vòng 14 ngày.

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
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/scripts/audit_security_mandates.py
  - category: implementation
    label: Lớp platform Windows — nơi duy nhất SendInput được phép tồn tại
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/rust/crates/sky_dispatch_win32/src/
  - category: distribution
    label: Hợp đồng phân phối — portable unsigned/cập nhật thủ công
    url: https://github.com/pumni/Sky-Auto-Player/tree/main/rust/crates/sky_updater
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

Script kiểm tra tự động (`scripts/audit_security_mandates.py`) quét cả source Python dưới
`src/` và source Rust dưới `rust/` ở mỗi lần push và pull request. Bất kỳ commit nào đưa
vào lệnh gọi API bị cấm (hook, đọc bộ nhớ, remote thread, debug attach) đều fail CI ngay.
Các ngoại lệ lịch sử, nếu có, được liệt kê trong `.config/security_audit_baseline.json`
với lý do giải thích và tham chiếu tracking.

Để chạy kiểm tra cục bộ:

```powershell
uv run --env-file .env python scripts/audit_security_mandates.py
```

## Mã nguồn

Sky Auto Player là mã nguồn mở theo GNU General Public License v3.0. Toàn bộ mã nguồn
có tại [github.com/pumni/Sky-Auto-Player](https://github.com/pumni/Sky-Auto-Player). Bạn
có thể kiểm tra code, build từ source, hoặc xác minh binary release khớp với source.

## Xác minh bản phát hành

Mỗi bản phát hành tạo ra ba file:

- `Sky-Auto-Player-v<phiên bản>.zip` — archive ứng dụng
- `Sky-Auto-Player-v<phiên bản>.zip.sha256` — checksum SHA256
- `MANIFEST.json` — manifest liệt kê checksum của mọi asset

Xác minh archive trước khi giải nén:

```powershell
(Get-FileHash "Sky-Auto-Player-v<version>.zip" -Algorithm SHA256).Hash
# So sánh với nội dung file .sha256
```

## Ranh giới cập nhật công khai

Binary Windows công khai được cố ý để unsigned và gói phát hành có native updater.
Updater chỉ chạy sau lựa chọn của người dùng, xác minh ZIP, SHA256 và `MANIFEST.json` chính xác
trước khi thay thế theo transaction; đường thủ công chỉ mở trang GitHub Releases chính thức.
Authenticode là **N/A — intentionally unsigned**.

Bằng chứng integrity/provenance của gói vẫn gồm:

- ZIP canonical, `.zip.sha256` và `MANIFEST.json`
- manifest băm đúng các byte unsigned được đóng gói
- build provenance/attestation của GitHub

Binary Rust `sky_updater` được đóng gói và chỉ được gọi từ thao tác update do người dùng chọn.
Nó vẫn là security component fail-closed và được test riêng về HTTPS, archive, manifest và transaction; không có
AuthentiCode requirement hay signature bypass thay vì thêm
bypass chữ ký.

## Thông báo Điều khoản Dịch vụ

Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky: Children of the Light.
Sky Auto Player là dự án cộng đồng không chính thức và không có liên kết hay sự chứng thực
từ thatgamecompany. Dùng có trách nhiệm và tự chịu rủi ro.

## Báo cáo lỗ hổng

Gửi email đến **pumni.dev@gmail.com**. Không mở issue công khai với các bước tái tạo.
Mong đợi xác nhận trong vòng 7 ngày và quyết định triage trong vòng 14 ngày.

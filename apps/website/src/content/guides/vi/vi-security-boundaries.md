---
key: security-boundaries
locale: vi
slug: security-boundaries
title: Ranh Giới Bảo Mật
description: >-
  Tóm tắt ranh giới input của Sky Auto Player và đường dẫn đến chính sách bảo mật chính thức.
summary: >-
  Gameplay input dùng Windows SendInput. Các quy tắc đầy đủ về bảo mật, release trust,
  audit và báo cáo lỗ hổng được duy trì trong SECURITY.md.
category: technical-safety
order: 1
published: '2026-08-08'
updated: '2026-11-26'
draft: false
related:
  - how-it-works
  - windows-setup
  - timing-engine
evidence:
  - category: security
    label: Chính sách bảo mật chính thức
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: security
    label: Security audit tĩnh
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/xtask/src/audits/security.rs
---

## Tóm tắt

Sky Auto Player gửi musical packet qua Windows SendInput chỉ sau khi kiểm tra mới foreground và identity của target đã giữ. Phần mềm không đọc hay sửa bộ nhớ hoặc file game, không inject code, không cài keyboard/input hook, không attach debugger và không bypass anti-cheat.

Foreground event callback đã được audit chỉ phát hint để kiểm tra lại target hiện tại; callback không thể cấp quyền gửi. Safety cleanup có giới hạn có thể thử nhả riêng các phím đã ghi trong journal.

## Chính sách chính thức

[SECURITY.md](https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md)
là nguồn chính thức cho toàn bộ ranh giới bảo mật, security audit, updater và
release trust, hành vi ngoài phạm vi và hướng dẫn báo cáo lỗ hổng. Hãy dùng
tài liệu đó khi review implementation hoặc báo cáo vấn đề.

Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky. Sky Auto
Player là dự án cộng đồng không chính thức; xem chính sách chính thức để đọc
đầy đủ thông báo và phạm vi.

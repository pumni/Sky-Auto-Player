---
key: how-it-works
locale: vi
slug: how-it-works
title: Sky Auto Player Hoạt Động Như Thế Nào
description: >-
  Tìm hiểu cách Sky Auto Player đọc file sheet, lên lịch nốt nhạc với timing căn theo frame,
  và gửi phím qua Windows SendInput API — mà không động đến game.
summary: >-
  Sky Auto Player đọc file sheet do người dùng cung cấp, lên lịch từng nốt nhạc theo mô hình
  timing, và gửi phím qua Windows SendInput API. Không đọc bộ nhớ game, không sửa file game,
  không inject code.
category: getting-started
order: 1
published: '2026-08-08'
updated: '2026-08-08'
draft: false
showDiagram: true
related:
  - sheet-formats
  - timing-engine
  - security-boundaries
evidence:
  - category: architecture
    label: Tổng quan kiến trúc bốn lớp
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/architecture.md
  - category: security
    label: Các mandate bảo mật (chỉ SendInput, không can thiệp game)
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: implementation
    label: README — tóm tắt workflow và kỹ thuật
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: architecture
    label: Kiến trúc dispatch thời gian thực
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/rt-dispatch-architecture.md
---

## Phần mềm làm gì

Sky Auto Player đọc file sheet nhạc do bạn cung cấp, lên lịch từng nốt, hợp âm và nốt ngân
theo một mô hình timing, rồi gửi phím đến cửa sổ đang active qua Windows `SendInput` API —
cùng kênh mà bất kỳ macro bàn phím thông thường nào cũng dùng. Phần mềm không đọc bộ nhớ
game, không kiểm tra file game, không sửa đổi game, không inject code, và không attach debugger.

Quy trình sử dụng điển hình:

1. Export sheet từ [Sky Music editor](https://specy.github.io/skyMusic/) dưới dạng JSON, `.skysheet`, hoặc TXT tương thích JSON.
2. Đặt file vào thư mục `songs/` cạnh `Sky-Auto-Player.exe`.
3. Khởi động Sky Auto Player, chọn bài, chuyển sang game và nhấn hotkey phát nhạc.

Bản đóng gói `Sky-Auto-Player.exe` là giao diện Tauri desktop chính thức, gồm Library, Song
Detail, Player Dock, Diagnostics, Settings và Update. `play.bat` cùng
`Sky-Auto-Player-Core.exe --tui` vẫn là các entry point fallback điều khiển bằng bàn phím.

Sky Auto Player gửi phím; game nhận chúng theo lịch của riêng nó. Độ chính xác timing
về phía game phụ thuộc vào lịch trình Windows, tần số polling input của game, và phần cứng
của bạn — không phải do player kiểm soát.

## Cách lên lịch nốt nhạc hoạt động

Sky Auto Player không phát lại một chuỗi delay cố định. Thay vào đó, nó đặt mỗi nốt như
một sự kiện trên timeline tiến theo tempo của sheet và FPS bạn cấu hình trong Sky.
Vòng lặp cốt lõi:

- **Chord batch liên tục** — tất cả nốt trong một hợp âm được gửi trong một lệnh `SendInput`
  duy nhất để giảm thiểu độ lệch phía sender giữa các phím trong cùng hợp âm.
- **Ngữ nghĩa hold** — nốt dài được giữ suốt thời gian ký âm đầy đủ. Nốt hold không bị
  cắt ngắn chỉ vì nốt tiếp theo đến sát.
- **Căn hold theo frame** — thời gian hold được biểu diễn dưới dạng 1,0; 1,25; hoặc 1,5
  frame game ở FPS bạn chọn. Mặc định là 1,0 frame.
- **Bù độ trễ** — scheduler đo lỗi completion phía sender trên máy hiện tại và điều chỉnh
  lead time cho phù hợp. Điều này giảm drift có hệ thống; không đảm bảo độ chính xác
  timing về phía game — vốn còn phụ thuộc vào Windows và vòng lặp sampling của game.

## Tổng quan kiến trúc

Code tuân theo thiết kế bốn lớp:

| Lớp                | Trách nhiệm                                                                         |
| ------------------ | ----------------------------------------------------------------------------------- |
| **Domain**         | Mô hình timing thuần túy, lịch, profile. Không I/O.                                 |
| **Orchestration**  | Scheduler và coordinator. Không wall-clock hay SendInput.                           |
| **Infrastructure** | Theo dõi focus, hotkey listener, real-time sleeper, đăng ký MMCSS.                  |
| **Platform**       | Backend Windows: `SendInput`, waitable timer, MMCSS. Nơi duy nhất chứa Win32 types. |

Một Rust timing worker sở hữu vòng lặp dispatch: biên dịch lịch nốt nhạc, timing wait độ phân giải cao, theo dõi focus, lệnh gọi `SendInput`, dọn dẹp và telemetry native. Tauri/React render desktop UI, còn Python Core sở hữu policy ứng dụng và orchestration dùng chung. Fallback Textual dùng các service đã tách ra này. Các đường presentation và dispatch chạy trên thread riêng; build Python 3.14 free-threaded đảm bảo vòng lặp dispatch không tranh chấp với UI trên GIL.

## Những gì Sky Auto Player KHÔNG làm

- Đọc hay ghi bộ nhớ của process khác
- Sửa đổi, vá lỗi, hay kiểm tra file game
- Inject DLL hay attach debugger vào bất kỳ process nào
- Cài đặt Windows hook (`SetWindowsHookEx`, v.v.)
- Bypass hệ thống chống gian lận
- Đảm bảo độ chính xác timing về phía game — timing phía sender được tối ưu, nhưng game
  quyết định khi nào đăng ký phím

## Giới hạn

- Chỉ hỗ trợ Windows 10 và 11 (64-bit). MMCSS là Windows API.
- Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky. Dùng có trách nhiệm và tự chịu rủi ro.
- Sky Auto Player là dự án cộng đồng không chính thức, không có liên kết hay sự chứng thực từ thatgamecompany.

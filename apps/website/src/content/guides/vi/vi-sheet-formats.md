---
key: sheet-formats
locale: vi
slug: sheet-formats
title: Các Định Dạng Sheet Được Hỗ Trợ
description: >-
  Sky Auto Player hỗ trợ file JSON, .skysheet và TXT tương thích JSON export từ Sky Music editor.
  Tìm hiểu nội dung từng định dạng và cách thêm bài hát đúng cách.
summary: >-
  Sky Auto Player đọc file JSON, .skysheet và TXT tương thích JSON được nhập qua Library.
  Cả ba định dạng đều xuất phát từ Sky Music editor và dùng chung cấu trúc dữ liệu.
category: getting-started
order: 2
published: '2026-08-08'
updated: '2026-11-26'
draft: false
related:
  - windows-setup
  - how-it-works
  - troubleshooting
evidence:
  - category: implementation
    label: README — định dạng được hỗ trợ và workflow
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: architecture
    label: Domain models — schedule và timing frame
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/rust/crates/sky_app_core/src/song.rs
---

## Tổng quan

Sky Auto Player đọc các file sheet bạn export từ [Sky Music editor](https://specy.github.io/skyMusic/)
(còn gọi là Sky Music Nightly). Ba định dạng được chấp nhận:

| Định dạng            | Phần mở rộng | Ghi chú                                 |
| -------------------- | ------------ | --------------------------------------- |
| JSON                 | `.json`      | Export mặc định từ editor               |
| Sky sheet            | `.skysheet`  | Định dạng native binary/JSON của editor |
| TXT tương thích JSON | `.txt`       | File văn bản chứa JSON hợp lệ           |

Cả ba dùng cùng cấu trúc dữ liệu — phần mở rộng xác định cách phát hiện file; parser đọc nội dung JSON bất kể phần mở rộng là gì.

## Đặt file bài hát ở đâu

Tạo hoặc chọn playlist trong Library desktop, sau đó chọn **Add songs** để nhập file hoặc cả thư mục. Thao tác nhập tham chiếu file gốc tại chỗ; ứng dụng không di chuyển hay xóa file. Nếu bạn di chuyển hoặc xóa file nguồn, mục đã nhập có thể không còn dùng được.

## Lấy file sheet ở đâu

1. Mở [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Tìm hoặc tạo một bản nhạc.
3. Dùng chức năng **Export** của editor để lưu dưới dạng JSON hoặc `.skysheet`.
4. Nhập file đã export qua Library desktop.

## Các lỗi thường gặp

**File không hiện trong Library**
: Kiểm tra file đã được nhập vào playlist và nguồn còn ở đường dẫn ban đầu. Phần mở rộng phải là .json, .skysheet, hoặc .txt; các định dạng khác không được nhận diện.

**Lỗi "Failed to parse"**
: Nội dung file không phải JSON hợp lệ. Mở file trong text editor để xác nhận nó chứa
một JSON object, không phải văn bản thường hay dữ liệu nhị phân.

**Bài hát phát nốt không đúng thứ tự hoặc bỏ qua đoạn**
: Sheet có thể chứa timing không khớp với FPS hoặc tempo đã cấu hình. Xem **Tempo** và FPS trong **Playback profile** của Player Dock. Lịch chạy Windows và vòng lấy mẫu input của game vẫn ảnh hưởng thời điểm phím được đăng ký.

## Playback profile

**Playback profile** trong Player Dock điều khiển cấu hình playback mặc định của ứng dụng; các giá trị này không được lưu riêng theo bài hay nhúng trong sheet. **Base Hold** có các giá trị 1, 1,25 và 1,5 frame; **Tempo** có 0,90×, 0,95×, 1,0×, 1,05× và 1,10×. Mục **Advanced timing** hỗ trợ FPS 30, 60, 90, 120, 144, 165 hoặc 240 và timing margin từ 0 đến 3.000 µs, bước 100 µs.

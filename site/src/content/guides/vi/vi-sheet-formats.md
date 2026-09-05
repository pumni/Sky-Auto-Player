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
updated: '2026-08-08'
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

Dùng Library desktop để nhập file sheet. Dữ liệu ứng dụng được lưu ở vùng app-data của người
dùng, tách khỏi thư mục installer và không được quản lý như file cạnh executable.

Sau khi thêm file, nhấn **Reload songs** trong Library desktop mà không cần khởi động lại ứng
dụng.

## Lấy file sheet ở đâu

1. Mở [Sky Music Nightly](https://specy.github.io/skyMusic/).
2. Tìm hoặc tạo một bản nhạc.
3. Dùng chức năng **Export** của editor để lưu dưới dạng JSON hoặc `.skysheet`.
4. Nhập file đã export qua Library desktop.

## Các lỗi thường gặp

**File không hiện trong Library**
: Dùng **Reload songs** trong GUI desktop.
Nếu vẫn không hiện, kiểm tra phần mở rộng file phải là `.json`, `.skysheet`, hoặc `.txt`.
Các phần mở rộng khác không được nhận diện.

**Lỗi "Failed to parse"**
: Nội dung file không phải JSON hợp lệ. Mở file trong text editor để xác nhận nó chứa
một JSON object, không phải văn bản thường hay dữ liệu nhị phân.

**Bài hát phát nốt không đúng thứ tự hoặc bỏ qua đoạn**
: Sheet có thể chứa dữ liệu timing không khớp với FPS hay tempo đã cấu hình. Thử điều
chỉnh tempo multiplier hoặc FPS trong cài đặt theo bài.

## Cấu hình theo bài

Sky Auto Player lưu cài đặt riêng cho từng bài (chọn hold frame, tempo, FPS, theme) tách biệt với file sheet. Thay đổi file sheet không reset các cài đặt này.

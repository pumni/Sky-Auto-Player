---
key: 'updates'
locale: 'vi'
order: 8
category: 'General'
question: "C\u01a1 ch\u1ebf c\u1eadp nh\u1eadt c\u1ee7a Sky Auto Player ho\u1ea1t \u0111\u1ed9ng th\u1ebf n\u00e0o?"
---

Ứng dụng đang chạy chỉ thông báo khi có bản phát hành mới. Đóng app và chạy `updater.bat` để cập nhật. Updater xác minh sidecar SHA256 của ZIP trước khi chạm vào file cài đặt, staging và kiểm tra bản phát hành, sao chép binary theo transaction có rollback, đồng thời bảo toàn `config.json`, `.env`, `songs/` và `logs/`.

Sau khi cập nhật binary thành công, chỉ các trường thông báo cập nhật trong `config.json` mới có thể được patch.

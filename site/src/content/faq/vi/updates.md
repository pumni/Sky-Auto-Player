---
key: 'updates'
locale: 'vi'
order: 8
category: 'General'
question: "C\u01a1 ch\u1ebf c\u1eadp nh\u1eadt c\u1ee7a Sky Auto Player ho\u1ea1t \u0111\u1ed9ng th\u1ebf n\u00e0o?"
---

Chọn **Update now** trong ứng dụng. App dừng playback, giải phóng phím, chép native updater
vào thư mục chạy riêng rồi thoát; updater tự tải đúng tag, kiểm tra ZIP/sidecar SHA256,
manifest và Authenticode trước khi cập nhật transaction có rollback. Updater luôn bảo toàn
`config.json`, `.env`, `songs/` và `logs/`.

Sau khi cập nhật binary thành công, chỉ các trường thông báo cập nhật trong `config.json` mới có thể được patch.

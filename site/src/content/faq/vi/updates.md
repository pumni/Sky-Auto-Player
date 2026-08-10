---
key: 'updates'
locale: 'vi'
order: 8
category: 'General'
question: "C\u01a1 ch\u1ebf c\u1eadp nh\u1eadt c\u1ee7a Sky Auto Player ho\u1ea1t \u0111\u1ed9ng th\u1ebf n\u00e0o?"
---

Ứng dụng kiểm tra phiên bản mới và cung cấp **Update and Restart**. Native updater đi kèm sẽ
tải đúng release, xác minh SHA256 của ZIP và `MANIFEST.json`, giữ nguyên `config.json`, `.env`,
`songs/` và `logs/`, sau đó thay thế file theo transaction rồi khởi động lại. Nếu không thể
stage update, hãy chọn **Open GitHub Releases** để tải ZIP canonical, `.zip.sha256` và
`MANIFEST.json` từ [trang GitHub Releases chính thức](https://github.com/pumni/Sky-Auto-Player/releases)
và cài vào thư mục mới mà người dùng có quyền ghi. Binary công khai được cố ý để unsigned.

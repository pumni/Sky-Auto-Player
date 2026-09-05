---
key: 'updates'
locale: 'vi'
order: 8
category: 'General'
question: "C\u01a1 ch\u1ebf c\u1eadp nh\u1eadt c\u1ee7a Sky Auto Player ho\u1ea1t \u0111\u1ed9ng th\u1ebf n\u00e0o?"
---

Ứng dụng kiểm tra channel v4 và cung cấp **Update and Restart**. Official Tauri updater qua
`UpdateService` Rust xác minh artifact NSIS đúng bằng `.exe.sig` rồi chạy installer current-user.
V4 không có custom updater executable, updater ZIP portable hoặc hợp đồng `MANIFEST.json.sig`.
Nếu update không thể stage, hãy tải installer canonical từ [v4 release authority](https://github.com/pumni/Sky-Auto-Player-Releases/releases).

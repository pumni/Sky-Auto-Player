---
key: 'source'
locale: 'vi'
order: 11
category: 'General'
question: "C\u00f3 th\u1ec3 build Sky Auto Player t\u1eeb m\u00e3 ngu\u1ed3n kh\u00f4ng?"
---

Có. Clone [repository](https://github.com/pumni/Sky-Auto-Player), chạy `uv sync`, sau đó dùng
`uv run python src/main.py` để khởi chạy fallback Textual/CLI từ source. GUI đóng gói được build
từ workspace Tauri trong `desktop/`; repository có tài liệu về môi trường Python 3.14
free-threaded và các kiểm tra doctor.

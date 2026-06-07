# Kế hoạch loại bỏ UI Classic cũ (prompt-toolkit)

Tài liệu này lập kế hoạch chi tiết để loại bỏ hoàn toàn giao diện Classic UI (sử dụng thư viện `prompt-toolkit`) cũ của **Sky Player**, dọn dẹp các module liên quan, và tối ưu hóa việc phân chia mã nguồn cho Textual UI mới.

---

## 1. Mục tiêu & Định hướng
- **Mục tiêu**: Loại bỏ dependency `prompt-toolkit`, giảm thiểu dung lượng bundle khi đóng gói, dọn dẹp mã nguồn cũ để tăng tính bảo trì.
- **Giải pháp chuyển tiếp**:
  - Giao diện chính mặc định sẽ là **Textual UI** (nếu terminal có hỗ trợ).
  - Terminal không hỗ trợ TTY / không tương thích sẽ tự động fall back về giao diện dòng lệnh cơ bản (Simple CLI mode - nhập số/tên bài qua `input()`), không cần qua `prompt-toolkit`.
  - Giữ lại các hằng số cấu hình chung trong [picker.py](file:///D:/Dev/Sky%20Player/src/sky_music/ui/picker.py) để tránh phá vỡ các import hiện tại ở các file khác.

---

## 2. Các tệp tin cần xoá bỏ hoàn toàn
Các module này chỉ phục vụ cho Classic UI và có thể được xoá bỏ an toàn:
1. **[src/sky_music/ui/picker_background.py](file:///D:/Dev/Sky%20Player/src/sky_music/ui/picker_background.py)**: Quản lý vòng đời tiến trình chạy nền của Classic UI.
2. **[src/sky_music/ui/picker_layout.py](file:///D:/Dev/Sky%20Player/src/sky_music/ui/picker_layout.py)**: Chứa các hàm vẽ hộp hiển thị dạng bảng kiểu cũ.
3. **[tests/test_classic_picker_lifecycle.py](file:///D:/Dev/Sky%20Player/tests/test_classic_picker_lifecycle.py)**: Bộ test kiểm thử luồng nền của Classic UI.

---

## 3. Các tệp tin cần sửa đổi

### A. [pyproject.toml](file:///D:/Dev/Sky%20Player/pyproject.toml)
- **Thay đổi**: Loại bỏ dependency `"prompt-toolkit>=3.0.52"`.
```diff
 dependencies = [
-    "prompt-toolkit>=3.0.52",
     "pyinstaller>=6.0.0",
     "rapidfuzz>=3.14.5",
     "textual>=8.2.7",
 ]
```

### B. [src/sky_music/ui/picker.py](file:///D:/Dev/Sky%20Player/src/sky_music/ui/picker.py)
- **Thay đổi**: Thu gọn tệp tin, loại bỏ toàn bộ logic giao diện của `prompt-toolkit` và chỉ giữ lại các cấu hình dùng chung mà các tệp khác vẫn import:
  - `SongPickerResult` (dataclass)
  - `ACTIVE_THEME`
  - `PROFILES_INFO`
  - `get_profiles_info`
  - `TEMPO_OPTIONS`
  - `FPS_OPTIONS`
- **Xoá**: `PickerState`, `choose_song_interactively`, `safe_exit` và các hàm helper phục vụ cho hiển thị cũ.

### C. [src/sky_music/ui/text_render.py](file:///D:/Dev/Sky%20Player/src/sky_music/ui/text_render.py)
- **Thay đổi**: Thay thế thư viện tính toán độ rộng ký tự Unicode của `prompt-toolkit` bằng hàm `rich.cells.cell_len` có sẵn (Textual phụ thuộc trực tiếp vào `rich`).
```diff
-try:  # prompt_toolkit is the production source of truth for cell width.
-    from prompt_toolkit.utils import get_cwidth as _pt_cwidth
-except Exception:  # pragma: no cover - fallback for non prompt_toolkit callers/tests
-    _pt_cwidth = None
+from rich.cells import cell_len as _pt_cwidth
```

### D. [src/main.py](file:///D:/Dev/Sky%20Player/src/main.py)
- **Thay đổi**:
  - Sửa đổi tham số CLI `--ui`:
    - Thay thế hoặc ẩn đi lựa chọn `classic` (nếu người dùng truyền `--ui classic`, chuyển tiếp hành vi hoặc báo lỗi/chuyển sang chế độ CLI `simple`).
    - Các lựa chọn mới: `auto`, `textual`, `simple`.
  - Trong hàm `prompt_song_selection`:
    - Loại bỏ nhánh kiểm tra `songs.HAS_PROMPT_TOOLKIT` và gọi `songs.choose_song_interactively(...)`.
    - Trực tiếp chuyển từ Textual UI sang giao diện CLI Simple (sử dụng loop `input()`) nếu Textual không được kích hoạt/hỗ trợ.
```diff
-    if PICKER_UI_MODE == "textual" or (PICKER_UI_MODE == "auto" and _supports_textual()):
+    if PICKER_UI_MODE == "textual" or (PICKER_UI_MODE == "auto" and _supports_textual()):
         from sky_music.ui.textual_app import choose_song_interactively_textual
         ...
-
-    if songs.HAS_PROMPT_TOOLKIT:
-        return songs.choose_song_interactively(...)
```

### E. [src/sky_music/ui/__init__.py](file:///D:/Dev/Sky%20Player/src/sky_music/ui/__init__.py)
- **Thay đổi**: Loại bỏ import và export cho hàm `choose_song_interactively`.

### F. [src/build_app.py](file:///D:/Dev/Sky%20Player/src/build_app.py)
- **Thay đổi**: Loại bỏ các module đã xoá khỏi danh sách `hidden_imports` phục vụ cho việc đóng gói bằng PyInstaller.
```diff
         # UI – classic prompt-toolkit picker and sub-modules
         "sky_music.ui.picker",
-        "sky_music.ui.picker_background",
         "sky_music.ui.picker_helpers",
-        "sky_music.ui.picker_layout",
         "sky_music.ui.picker_metadata",
```

### G. [README.md](file:///D:/Dev/Sky%20Player/README.md)
- **Thay đổi**: Cập nhật phần hướng dẫn sử dụng tham số `--ui`, chỉ rõ mặc định là `auto` (sử dụng Textual) và fallback là chế độ dòng lệnh đơn giản (Simple CLI).

### H. [docs/INDEX.md](file:///D:/Dev/Sky%20Player/docs/INDEX.md)
- **Thay đổi**: Đánh dấu tài liệu kế hoạch Textual cũ là hoàn thành/lưu trữ và đưa tài liệu kế hoạch loại bỏ Classic UI này vào trạng thái lưu trữ khi hoàn tất.

---

## 4. Kế hoạch kiểm thử & Xác thực
1. **Cập nhật lockfile**: Chạy `uv sync` để cập nhật `uv.lock` và gỡ bỏ hoàn toàn gói `prompt-toolkit`.
2. **Chạy kiểm thử tự động**:
   - Chạy `uv run pytest` để xác nhận tất cả 280+ testcase (ngoại trừ các testcase của classic UI đã bị xoá) đều tiếp tục PASS.
   - Đảm bảo `tests/test_textual_picker.py` hoạt động hoàn hảo.
3. **Kiểm thử thủ công**:
   - Chạy thử ứng dụng bằng lệnh:
     - `uv run python src/main.py --ui textual` (Kiểm tra Textual UI hoạt động bình thường).
     - `uv run python src/main.py --ui simple` (Kiểm tra giao diện dòng lệnh đơn giản).
4. **Xác thực đóng gói**:
   - Chạy lệnh `uv run build-app` để biên dịch ứng dụng và đảm bảo không phát sinh lỗi đóng gói do thiếu hidden imports hay dependencies.

# Sky Auto Player — Clean Self-Update / Defender Hardening Plan

## Mục đích

Bộ tài liệu này là handoff dành cho AI coding để chuyển Sky Auto Player sang kiến trúc phát hành sạch:

- **Một ZIP duy nhất** cho mỗi release.
- **Không hỗ trợ legacy updater**.
- Xóa hoàn toàn `updater.bat` và `installer/updater.ps1` khỏi kiến trúc đích.
- Ứng dụng có nút **Update now**.
- Update được thực hiện bởi **native Rust updater** chạy ngoài process chính.
- Updater có `download → verify → stage → transactional install → rollback → restart`.
- Giữ ứng dụng portable: không MSI, không registry installer, không yêu cầu admin trong thư mục user-writable.
- Giảm false-positive surface của Windows Defender bằng:
  - thay polling `GetAsyncKeyState` cho hotkey bằng `RegisterHotKey`;
  - giảm aggressive window-focus APIs;
  - xóa PowerShell/BAT downloader/updater khỏi distribution;
  - Authenticode-sign các PE do project sở hữu;
  - tạo manifest sau signing;
  - dùng PyInstaller bootloader build từ source;
  - giảm collection surface trong PyInstaller.

## Baseline

Kế hoạch được lập dựa trên:

- Repository: `pumni/Sky-Auto-Player`
- Branch: `main`
- Commit: `c2f0f573e5087684f4c641a5f2ba4abb478e897c`
- Version hiện tại trong `pyproject.toml`: `3.1.0`

AI coding **phải kiểm tra HEAD trước khi sửa**. Nếu HEAD đã thay đổi, phải đọc lại source hiện tại và điều chỉnh patch; không áp dụng line-number cứng từ kế hoạch.

## Quyết định đã chốt

Đây là **clean cutover**.

Không tạo:

- `legacy.zip`;
- `portable.zip` song song;
- bridge package;
- `Sky-Player.exe` compatibility;
- PowerShell updater fallback;
- BAT launcher fallback;
- dual-name resolution;
- migration code cho updater cũ.

Release đầu tiên của kiến trúc mới có thể yêu cầu người dùng đang ở bản cũ **download thủ công một lần**. Không xây compatibility code chỉ để tránh bước này.

Không được tuyên bố hoặc test như một contract rằng updater cũ có thể upgrade lên release mới.

## Cách dùng

1. Gửi `01_AI_CODING_HANDOFF.md` cho coding agent.
2. Coding agent phải đọc `AGENTS.md`, `SECURITY.md`, normative docs và source thật.
3. Thực hiện theo `03_PHASED_IMPLEMENTATION_PLAN.md`.
4. Đặc tả updater ở `02_TARGET_ARCHITECTURE_AND_UPDATER_SPEC.md`.
5. Build/sign/package theo `04_BUILD_SIGNING_PACKAGING.md`.
6. Hotkey/focus theo `05_HOTKEY_FOCUS_HARDENING.md`.
7. Validation theo `06_TEST_VALIDATION_MATRIX.md`.
8. Dùng `07_CLEAN_CUTOVER_TREE_AND_REPORT.md` để kiểm tra cutover và báo cáo từng phase.

## Invariant không được phá

- P0 của repository giữ nguyên.
- Chỉ dùng `SendInput` để phát input.
- Không hook bàn phím (`SetWindowsHookEx` bị cấm).
- Không đọc/ghi memory process game.
- Không inject DLL.
- Không bypass anti-cheat.
- Không obfuscate/pack/junk-code để né AV.
- Không disable Defender.
- Không thêm Defender exclusion.
- Không làm yếu hash/signature/manifest checks.
- Không chạm semantics của Rust real-time dispatch nếu không bắt buộc.

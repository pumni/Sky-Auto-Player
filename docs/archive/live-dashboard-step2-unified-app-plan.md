# Kế hoạch: Unified Control Center (Bước 2) — bản thực thi

> **Đối tượng đọc:** kỹ sư **thực thi**. Tài liệu tự chứa — không cần ngữ cảnh hội thoại.
> **Reviewer:** tác nhân giám sát (nghiệm thu theo "Cổng kiểm duyệt" cuối tài liệu). Không merge khi chưa qua cổng.
> **Tiền đề:** Bước 1 đã đóng dấu (`live-playback-dashboard-plan.md`) — đã có `PlaybackApp`/`SnapshotRenderer` (snapshot + poll 10Hz). Bước 2 xây trên đó.
> **Ngày:** 2026-06-07. Sự thật hiện hành: `architecture.md`, `timing-principles.md`.

---

## 0. TL;DR

Bước 1: picker (Textual, `app.run()` #1) → **thoát** → `play_selected_song` in **console** (risk prompt / countdown / error bằng `print`) → playback (Textual, `app.run()` #2) → quay lại picker. Giữa hai app vẫn còn **"console đen"**.

Bước 2 gộp thành **MỘT `app.run()` bền vững** sở hữu cả vòng đời phiên: PickerScreen → (RiskModal/ErrorModal nếu cần) → CountdownScreen → PlaybackScreen → về PickerScreen. **Xoá hẳn console đen** lúc tương tác: mọi risk/countdown/error chuyển sang screen/modal Textual.

**Phạm vi chỉ áp dụng cho luồng tương tác TTY.** Luồng `--song <name>` trực tiếp và non-TTY **giữ nguyên** `play_selected_song` console hiện tại (bất biến CLI). Debug panel (lateness/jitter/FPS) vẫn là **Bước 3, ngoài phạm vi**.

---

## 1. Bất biến (vi phạm = fail review ngay)

1. **KHÔNG sửa** file trong: `src/sky_music/domain/`, `src/sky_music/orchestration/` (đặc biệt `engine.py`, `runtime_dispatch.py`, `telemetry.py`), `src/sky_music/infrastructure/`, `src/sky_music/platform/`, `src/sky_music/ui/hud.py`, `src/sky_music/config.py`, `src/sky_music/layouts.py`. Được **đọc & gọi**, không **đổi**.
2. **KHÔNG đụng** đường dispatch real-time, scheduler, GC-pause, hay bất kỳ cờ timing truyền vào `PlaybackEngine`. `SnapshotRenderer` (Bước 1) **không đổi hợp đồng**.
3. **Giữ tương thích CLI tuyệt đối:** `--song`, `--list`, `--dry-run`, `--repeat`, mọi flag timing, `--inspect-telemetry`, `--auto-calibrate`, `--doctor*`, `--selftest-textual` hoạt động y như cũ. Luồng `--song` trực tiếp và non-TTY **không** đi qua app gộp — giữ `play_selected_song` cũ.
4. **KHÔNG regress picker đã ship:** mọi test trong `tests/test_textual_picker.py` phải vẫn xanh. Hành vi picker (fuzzy, metadata 2 tầng, theme, modal, calibration, reload) giữ nguyên.
5. **`SongPickerResult`** giữ nguyên shape (đang là hợp đồng nội bộ; nay tiêu thụ trong-process thay vì qua `app.exit`).
6. Coding standards (AGENTS.md): Python 3.14, **type hints bắt buộc**, `@dataclass(frozen=True, slots=True)` ưu tiên, không global mới, có test. Dùng `uv run`.

---

## 2. Bối cảnh kiến trúc (đọc kỹ trước khi code)

### 2.1 Hiện trạng (sau Bước 1)
- `main()` (main.py:1378): xử lý args → nếu `--song` → vòng lặp `play_selected_song` trực tiếp; nếu tương tác → vòng lặp `while True:` gọi `prompt_song_selection()` (chạy `SkyPickerApp.run()`, **thoát** trả `SongPickerResult`) → `play_selected_song()`.
- `prompt_song_selection` (main.py:1293): nếu `_check_textual_support()` báo lỗi → in box lỗi + `_wait_key_and_exit`; ngược lại chạy `choose_song_interactively_textual(...)` (SkyPickerApp).
- `play_selected_song` (main.py:565): **trộn logic + console I/O**:
  - Logic thuần: `get_shared_song_repository().load`, `build_key_actions`, `validate_key_actions`, `analyze_schedule`.
  - Console I/O cần loại bỏ ở luồng gộp: `print()` lỗi parse/`ScheduleBuildError`/violations; `_handle_risk_analysis(...)` (main.py:102, prompt console); `_mini_preflight(...)` (main.py:160); `countdown_before_playback(...)`.
  - Bước 1 đã thêm nhánh: nếu `_check_textual_support() is None` → `run_playback_textual(engine, SnapshotRenderer(), ...)`; ngược lại `ProgressRenderer` + `engine.play()`.
- `SkyPickerApp(App)` (`ui/textual_app/app.py`): teardown background workers ở `on_unmount` (`picker_scope.close_all(wait=True)` + `assert_closed`) — **điểm bảo đảm cách ly timing hiện tại**.
- `PlaybackApp(App)` (`ui/textual_app/playback_app.py`): chạy `engine.play()` ở `@work(thread=True)`, poll 10Hz.

### 2.2 ⚠️ Vì sao "console đen" KHÔNG thể tồn tại trong app gộp
Khi một App Textual đang chạy, nó chiếm **alt-screen** của terminal. Gọi `print()`/`input()` lúc đó sẽ **phá vỡ render**. Do đó trong app gộp, **mọi** risk-prompt / countdown / error phải là screen/modal Textual — không có đường lai "tạm giữ console".

### 2.3 ⚠️ Bảo đảm cách ly timing phải được CHUYỂN GIAO thủ công
Hôm nay picker workers tự teardown nhờ `on_unmount` khi `SkyPickerApp.run()` kết thúc. Trong app gộp, PickerScreen **không unmount** khi chuyển sang PlaybackScreen (screen chỉ bị che trong stack). Vì vậy **phải quiesce tường minh** toàn bộ background work của picker (MetadataCoordinator: thread worker + `ProcessPoolExecutor`) **trước khi** vào playback, và re-arm khi quay lại. Đây là yêu cầu an toàn timing **bắt buộc** (memory: `realtime-process-isolation`, `player-dispatch-proven-metronomic`).

---

## 3. Kiến trúc đích

```
SkyApp(App[int])                       # 1 app.run() cho cả phiên tương tác
├── PickerScreen(Screen)               # bọc composition picker hiện có (tái dùng tối đa)
│     └─ MetadataCoordinator (quiesce khi rời, re-arm khi về)
├── controller (thuần, không I/O):     # §3.1
│     prepare_playback(song_path, session, cfg) -> PlaybackPlan | PlaybackError
├── ErrorModal(ModalScreen)            # thay print lỗi parse/build/violations
├── RiskModal(ModalScreen)             # thay _handle_risk_analysis prompt
├── CountdownScreen(Screen)            # thay countdown_before_playback
└── PlaybackScreen(Screen)             # chuyển từ PlaybackApp (Bước 1) → Screen
```

Vòng đời trong **một** `app.run()`:
1. PickerScreen → user chọn → `SongPickerResult`.
2. `prepare_playback(...)` (thuần): load + build + validate + analyze. Lỗi fatal → `PlaybackError` → **ErrorModal** → về picker.
3. Nếu `report.severity != low` và chưa pre-decide → **RiskModal** (proceed / đổi profile-tempo / cancel). Áp dụng = rebuild qua controller.
4. **Quiesce picker workers** (§2.3).
5. **CountdownScreen** (đợi focus game; bỏ qua nếu dry-run).
6. **PlaybackScreen** chạy `engine.play()` (cơ chế Bước 1) → trả `finished|skipped|quit`.
7. Pop về PickerScreen, **re-arm** metadata; lặp. `quit` → `app.exit()`.

### 3.1 Controller thuần (tách logic khỏi I/O) — `ui/textual_app/playback_controller.py` (mới)
Trích phần **logic** của `play_selected_song` thành hàm/khối **không print, không input, không Textual**, unit-test được:
```python
@dataclass(frozen=True, slots=True)
class PlaybackPlan:
    engine_inputs: ...      # actions/sched_meta/session/policy đủ để dựng PlaybackEngine
    risk_report: ScheduleAnalysis
    session: PlaybackSessionContext

@dataclass(frozen=True, slots=True)
class PlaybackError:
    code: str
    message: str
    recommended_tempo_scale: float | None = None
    recommended_profile: str | None = None

def prepare_playback(song_path, session, cfg) -> PlaybackPlan | PlaybackError: ...
def rebuild_with(plan_or_session, *, profile=None, tempo=None) -> PlaybackPlan | PlaybackError: ...
```
> Không tái hiện lại logic — **gọi lại** `build_key_actions`, `validate_key_actions`, `analyze_schedule`, `get_shared_song_repository` y như `play_selected_song`. Mục tiêu: cùng kết quả, chỉ bỏ I/O. `play_selected_song` (luồng `--song`/non-TTY) **giữ nguyên** — có thể refactor để **dùng chung** controller nhưng KHÔNG bắt buộc ở Bước 2; nếu dùng chung phải có test chống lệch hành vi.

### 3.2 Quiesce/Resume picker workers
- Thêm API tường minh trên picker (qua `picker_scope`/`MetadataCoordinator`) để **đóng băng** trước playback và **re-arm** sau:
  - `quiesce()`: huỷ job đang chạy, đóng ProcessPool/thread, chờ `assert_closed()` — tương đương `on_unmount` hiện tại nhưng gọi chủ động.
  - `rearm()`: dựng lại coordinator + refresh metadata khi quay về picker.
- **Cấm**: bất kỳ ProcessPool/metadata thread nào sống trong lúc PlaybackScreen active. Reviewer sẽ kiểm bằng telemetry `last_picker_cleanup` + đọc code.

### 3.3 main() đổi gì
- Luồng tương tác TTY: thay vòng `while True:` gọi picker+play rời rạc bằng **một** `SkyApp(...).run()`. App tự lặp nội bộ (picker→play→picker).
- Luồng `--song` trực tiếp và non-TTY: **không đổi** (vẫn `play_selected_song`).
- `_check_textual_support()` vẫn là cổng vào app gộp; non-TTY giữ hành vi hiện tại.

---

## 4. An toàn timing (khử rủi ro bằng thiết kế)

| Nguy cơ | Biện pháp |
|---|---|
| Worker picker chạy lúc phát | `quiesce()` tường minh trước PlaybackScreen + `assert_closed`; re-arm khi về (§3.2) |
| UI làm bẩn đường gửi | Không đụng dispatch/scheduler/GC-pause; PlaybackScreen giữ cơ chế Bước 1 (snapshot + poll 10Hz, `call_from_thread` chỉ lúc exit) |
| Tranh hotkey F8/F9/F10 | Giữ như Bước 1 — global hook, app không bind |
| Regress picker đã ship | Tái dùng composition/logic picker; `tests/test_textual_picker.py` phải xanh (§1.4) |

> Tham chiếu memory: `realtime-process-isolation`, `player-dispatch-proven-metronomic`, `live-dashboard-decision`.

---

## 5. Phase nội bộ (tuần tự, mỗi phase 1 PR + gate)

### Phase 2.1 — Controller thuần (rủi ro thấp, nền tảng)
- Tạo `playback_controller.py` (§3.1). Trích logic, KHÔNG I/O.
- Test đơn vị: cùng input → `PlaybackPlan` khớp kết quả `play_selected_song` (song sạch); song lỗi → `PlaybackError` đúng code; risk severity đúng; rebuild đổi profile/tempo đúng.
- **Chưa** đổi `main()`/`play_selected_song`. Gate 2.1: controller xanh test, bất biến §1 rỗng.

### Phase 2.2 — App gộp (rủi ro cao)
- `SkyApp` + PickerScreen + ErrorModal + RiskModal + CountdownScreen + PlaybackScreen; quiesce/rearm (§3.2); đổi `main()` luồng tương tác.
- Test (`App.run_test()` + Pilot, engine giả như Bước 1):
  - picker→chọn→playback(engine giả finish)→về picker, trong **một** app.
  - song lỗi → ErrorModal → về picker (không crash, không console).
  - risk cao → RiskModal: proceed / cancel / đổi profile→rebuild.
  - **quiesce**: trước khi vào PlaybackScreen, `last_picker_cleanup.ok == True` và không còn worker chạy.
  - `quit` từ playback → app.exit; `skipped`/`finished` → về picker.
  - KHÔNG test nào chạy playback thật/SendInput/hotkey global.
- Gate 2.2: xem §6.

---

## 6. Cổng kiểm duyệt (reviewer chạy)

**Gate 2.1:** ✅ **ĐÓNG DẤU (2026-06-07, vòng 2)**
- [x] `git diff --name-only`: chỉ `ui/textual_app/playback_controller.py` (mới) + test. `M src/main.py` là dư Bước 1 chưa commit, không phải 2.1 (verify: main.py không tham chiếu controller). Bất biến §1 rỗng.
- [x] Controller không import Textual, không `print`/`input`. 7 unit test xanh (success/build_failed/validation_failed/rebuild plan+session/high_risk/dry_run_with_violations). Tái dùng domain funcs → parity-by-construction.
- [x] ruff sạch (All checks passed). Full suite: 2 fail đều non-regression — `test_rank_song_choices_benchmark` (98<100 bài, môi trường) + `test_live_cli_execution` (flaky dưới tải full-suite; chạy riêng PASS; controller không nằm trong đường CLI).
- [x] Vòng 1 trả lại: 5 unused import + mất non-fatal violations → đã vá: thêm `PlaybackPlan.violations: tuple[ScheduleInvariantViolation, ...]` lưu đủ cả non-fatal cho 2.2 surface.
> Nợ nhỏ mang sang (non-blocker): `prepare_playback` bắt `(ScheduleBuildError, ValueError)` rộng hơn `play_selected_song` (chỉ `ScheduleBuildError`) — có thể che bug; cân nhắc thu hẹp khi wire 2.2.

**Gate 2.2:**
- [ ] `git diff --name-only`: chỉ chạm `main.py`, `ui/textual_app/*` (app/playback/picker host + modals/screens mới). **Bất biến §1 rỗng** (đặc biệt `engine.py`, `hud.py`, domain/orchestration/infra/platform/config).
- [ ] **Cách ly timing:** đọc code xác nhận `quiesce()` chạy trước PlaybackScreen, không ProcessPool/metadata-thread nào sống lúc phát; telemetry `last_picker_cleanup.ok == True`. Vi phạm = chặn.
- [ ] **Không console khi Textual chạy:** không còn `print`/`input` trên đường tương tác gộp (risk/countdown/error đều Textual).
- [ ] **Regress picker:** `tests/test_textual_picker.py` xanh toàn bộ.
- [ ] **CLI bất biến:** `--song <name>`, `--list`, non-TTY, `--dry-run`, `--repeat` hành vi y cũ (regression smoke mô tả trong PR).
- [ ] Pilot test app gộp xanh; `uv run pytest` không tăng fail; ruff sạch; (pyright nếu chạy được).
- [ ] **Smoke phát thật (chủ dự án):** 1 lượt — chọn bài → (nếu có) RiskModal → countdown Textual → dashboard → F8/F9/F10 đúng → về picker, **không thấy console đen**.

## 6c. Kết quả nghiệm thu Gate 2.2 (reviewer, 2026-06-07)

**Trạng thái: ✅ ĐÓNG DẤU — BƯỚC 2 HOÀN TẤT (2026-06-07). Chủ dự án nghiệm thu smoke phát thật in-game OK. P2.1 + P2.2 đều qua cổng.**

- [x] Phạm vi file: `main.py` + `textual_app/{app,playback_app,modals,__init__}.py` + test. Bất biến §1 rỗng (engine/hud/domain/orchestration/config không đụng; `modals.py` chỉ gỡ debug print).
- [x] **Blocker timing-isolation:** `execute_playback_plan` kiểm `_picker_cleanup_failed` sau `quiesce()` → nếu fail: rearm + Cleanup modal + **return, không phát**. Khôi phục bất biến Bước 1.
- [x] **Double-entry guard** `_transitioning_to_playback`: set ở `action_confirm`, reset đủ mọi đường thoát (PlaybackError/risk lỗi/cancel/cleanup-fail/kết thúc playback).
- [x] **Parity:** cờ engine lấy động từ `RUNTIME_STATE` qua `_get_main_module()` (thử `__main__`→`import main`→None, fallback an toàn); non-fatal `plan.violations` → banner PlaybackScreen; risk modal wired `rebuild_with` (proceed/switch_profile/scale_tempo/dry_run/cancel); `scale_tempo` dùng `suggested_tempo_scale` động (analyzer.py:25).
- [x] ruff All checks passed; full suite 311 passed / 1 failed (benchmark env; `test_live_cli_execution` flaky đã pass lượt này); `test_textual_picker` không regress.
- [x] **UI state-machine luồng gộp:** verify bằng Pilot mock (picker→risk 5 lựa chọn→cancel→picker→proceed→countdown→dashboard→F9→picker→reselect→F10 exit).
- [ ] **Smoke phát THẬT in-game:** Pilot vừa rồi MOCK `prepare_playback`+`PlaybackEngine` → KHÔNG phủ quiesce worker thật / hotkey global thật / timing thật khi app sống xuyên suốt. **Chủ dự án tự chạy `uv run python src/main.py` (không --song)** xác nhận trước khi đóng dấu Bước 2.
> Note non-blocker: `modals.py` từng lọt debug `print()` (đã gỡ) — hygiene.

## 7. Quy trình bàn giao
1. Executor mở **1 PR/phase** (2.1 trước, 2.2 sau), kèm: mô tả việc, `pytest` đếm pass/fail, mô tả smoke (lệnh + quan sát), xác nhận "không chạm bất biến §1".
2. Reviewer chạy checklist gate tương ứng + `git diff --name-only`.
3. Chỉ qua Gate 2.1 mới làm 2.2. Gate 2.2 cần smoke thật của chủ dự án (như Bước 1) trước khi đóng.

## 8. Ngoài phạm vi (Bước 3)
Debug panel: lateness p50/p95, jitter, dropped, send_duration, FPS, timer resolution, MMCSS status, queue depth — toggle Normal/Debug. `max_lateness_us` đã tích luỹ sẵn ở `SnapshotRenderer` (Bước 1) chờ hiển thị.

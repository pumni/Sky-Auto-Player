# Main-path cleanup + build-quality plan (Python 3.14)

Status: PROPOSED — ready for an executing AI.
Owner of acceptance: Claude (nghiệm thu theo §6 gates).
Scope author context: viết sau khi `scripts/analyze_send_split.py` đã chứng minh **send-path đã tối ưu
tới hạn** (xem `send-split-analysis-gate` memory + run "Comedy" 20260623-130951: bookkeeping p50=10µs,
không có warmth penalty, send tail p99=953µs/max=1695µs). **Plan này KHÔNG phải tối ưu hiệu năng** — nó
là *vệ sinh mạch chính* + *chất lượng bản build* cho nền 3.14.

---

## 0. Hard constraints (đọc trước khi sửa gì)

- **KHÔNG đụng send-path / timing semantics.** Không sửa lead, floor, spin, coordinator, onset model.
  Đo đã chốt: không còn mỡ để nạo (`docs/rt-dispatch-architecture.md`, baseline §3).
- Tuân thủ `AGENTS.md`: `SendInput` only, không process priority class, type hints bắt buộc,
  `uv run` cho mọi lệnh, đổi nhỏ + có test, không rewrite rộng.
- **Đính chính nền tảng (quan trọng):** stock CPython **3.14 VẪN có GIL**. Chỉ bản free-threaded
  `python3.14t` mới tắt GIL. Do đó knob GIL switch-interval **không phải "config 3.13 lỗi thời cần
  xóa"** — nó vẫn có lợi trên stock 3.14, và chỉ trở thành no-op khi chạy free-threaded. Mục tiêu là
  làm nó **tự nhận biết môi trường**, không phải gỡ bỏ.
- Mọi knob môi trường phải **đúng mặc định trên cả hai build mà không cần chỉnh tay**; phần tinh chỉnh
  cho môi trường lạ chuyển thành **preset tài liệu cho người fork**, không phải default trong code nóng.

Mã nền hiện tại để tham chiếu:
- Switch-interval knob: `src/sky_music/infrastructure/realtime.py:56-61` (rationale + `DISPATCH_SWITCH_INTERVAL_S`),
  `:111-117` (`RealtimeProcessScope.__enter__` áp dụng `setswitchinterval`), `:131-135` (revert).
- Wiring: `RuntimeSessionState.enable_switch_interval_tuning` (`orchestration/runtime_session.py:29`)
  ← `main.py:472` (`not args.no_switch_interval_tuning`) → `engine.py:158,374` → realtime.
  Cũng dùng ở `cli/console_playback.py:559` và `ui/textual_app/app.py:1226-1271`.
- Kill switch CLI: `--no-switch-interval-tuning` (`main.py:303-307`); họ `--no-*` còn lại `main.py:285-323`.
- Doctor env report: `infrastructure/doctor.py:109-110`.
- Build: `Sky-Player.spec`, `src/build_app.py`, `pyproject.toml` (`requires-python = ">=3.11,<3.15"`,
  version 2.2.2), `.python-version = 3.14`.

---

## 1. Pillar A — Mạch chính tự nhận biết GIL (self-aware, không cần chỉnh tay)

**Vấn đề:** `RealtimeProcessScope` luôn gọi `sys.setswitchinterval(0.001)` khi
`enable_switch_interval_tuning=True`. Trên free-threaded build, `setswitchinterval` không còn ý nghĩa
(không có GIL để handoff) — chạy nó là no-op gây nhiễu (và telemetry báo "đã tune" sai sự thật).

### A1 — Gate switch-interval theo năng lực GIL
File: `src/sky_music/infrastructure/realtime.py`, trong `__enter__` (khối `:111-117`).

Trước khi `setswitchinterval`, kiểm tra GIL có bật không, an toàn cho cả <3.13 (không có hàm):

```python
def _gil_enabled() -> bool:
    # sys._is_gil_enabled() tồn tại từ 3.13; trên bản cũ hơn coi như có GIL.
    probe = getattr(sys, "_is_gil_enabled", None)
    return bool(probe()) if probe is not None else True
```

Sửa nhánh tuning:
```python
if self._enable_switch_interval_tuning and _gil_enabled():
    self._old_switch_interval = sys.getswitchinterval()
    sys.setswitchinterval(DISPATCH_SWITCH_INTERVAL_S)
    inputs.debug_log(f"[realtime] GIL switch interval tuned to {DISPATCH_SWITCH_INTERVAL_S}s")
elif self._enable_switch_interval_tuning:
    inputs.debug_log("[realtime] free-threaded build: switch-interval tuning skipped (no GIL)")
else:
    inputs.debug_log("[realtime] GIL switch interval tuning disabled")
```
`__exit__` không đổi: chỉ revert khi `_old_switch_interval is not None` (đã đúng ở `:131-135`), nên
khi skip thì không revert nhầm.

### A2 — Telemetry phản ánh trạng thái thật
Engine ghi `runtime_options["switch_interval_tuning"]` từ flag (`engine.py:190`). Thêm sự thật môi
trường để nghiệm thu phân biệt "tắt do cờ" vs "tắt do free-threaded":
- Trong `engine.py` chỗ build `runtime_options`, thêm key `"gil_enabled": _gil_enabled()` (đặt helper
  `_gil_enabled` ở `realtime.py` rồi import, để một nguồn sự thật duy nhất — đừng nhân bản).

### A3 — Doctor hiển thị trạng thái GIL/free-threaded
File: `infrastructure/doctor.py:109-110`. Sau dòng "Python Version", thêm:
```python
gil = getattr(sys, "_is_gil_enabled", None)
gil_state = "enabled" if (gil is None or gil()) else "DISABLED (free-threaded)"
print(f"GIL State        : {gil_state}")
```
Mục đích: người fork chạy `--doctor` biết ngay build của mình thuộc preset nào (§2).

### A4 — Tests
- `tests/test_realtime_scope.py` (đã tồn tại): thêm case mock `sys._is_gil_enabled` trả `False` →
  xác nhận `setswitchinterval` KHÔNG được gọi và `_old_switch_interval is None` (không revert). Và case
  trả `True` → gọi đúng như cũ. Dùng monkeypatch, không phụ thuộc build thật.
- Không thêm phụ thuộc mới.

**Lưu ý cho AI thực thi:** đây KHÔNG phải "xóa config 3.13". Trên stock 3.14 (có GIL) nhánh tuning vẫn
chạy y như cũ — gate chỉ đổi hành vi trên `3.14t`. Đừng đổi default `enable_switch_interval_tuning`.

---

## 2. Pillar B — Preset cho người tải/fork (externalize, không phải code)

Các kill switch đã tồn tại đầy đủ (`--no-*`, `--rt-priority-mode`). "Để dạng cho người fork" =
**một bảng preset tài liệu**, không thêm code/flag mới.

### B1 — Tạo `docs/tuning-presets.md`
Bảng map môi trường → cờ cần bật, chỉ dùng cờ đã có. Tối thiểu các preset:

| Preset | Khi nào | Lệnh |
|---|---|---|
| **Default (stock 3.14, có GIL)** | Đa số người dùng | (không cờ — mặc định đã tối ưu) |
| **Free-threaded (`3.14t`)** | Forker dùng build no-GIL | (không cờ; switch-interval tự skip nhờ §A1) |
| **Máy yếu / không MMCSS** | MMCSS fail trong telemetry `rt_priority_acquired=off` | `--rt-priority-mode highest` |
| **Debug jitter** | Điều tra nấc | `--no-event-wait` rồi so telemetry |
| **Tương thích tối đa** | Sleeper/timer lạ | `--no-waitable-timer --no-event-wait` |

Mỗi preset kèm 1 câu *vì sao* và *cách kiểm chứng bằng telemetry* (`--inspect-telemetry`). Nguồn sự thật
cho danh sách cờ: argparse `main.py:285-323` — liệt kê đúng tên cờ, đừng bịa cờ không có.

### B2 — README trỏ tới preset
Thêm một mục ngắn "Tuning for your machine / forks" trong `README.md` trỏ `docs/tuning-presets.md` và
nhắc chạy `--doctor` để biết GIL state. Không chép nội dung bảng vào README (tránh trùng lặp lệch nhau).

---

## 3. Pillar C — Chất lượng bản build trên 3.14

### C1 — Audit assert "gánh nặng" TRƯỚC khi cân nhắc `--optimize`
PyInstaller có thể build với bytecode optimize (`-OO`) → bỏ `assert` và `__doc__`, nhỏ + nhanh hơn.
**Nhưng codebase có assert mang ngữ nghĩa an toàn runtime**, nếu strip sẽ mất guard:
- `platform/win32/inputs.py:444` — `send_scan_code_batch_trusted` assert không trùng scan code.
- `orchestration/telemetry.py:250-257` — assert field bắt buộc.
Bước:
1. `rg -n "assert " src/` liệt kê toàn bộ.
2. Phân loại: (a) invariant debug thuần (an toàn strip) vs (b) guard gánh nặng (phải giữ).
3. Với loại (b) trên hot-path (vd inputs.py:444): chuyển ngữ nghĩa guard sang dạng không bị strip nếu
   muốn build `-OO` — HOẶC kết luận giữ `--optimize 1` (chỉ bỏ `__debug__`/docstring, **giữ assert**) /
   không optimize. **Khuyến nghị mặc định: `--optimize 1`** (an toàn, vẫn nhỏ hơn) trừ khi audit chứng
   minh mọi assert (b) đã được thay bằng guard thật.
4. Ghi quyết định vào plan này (mục "Decisions") kèm lý do.

Cách bật trong build: `Sky-Player.spec` — `Analysis(..., optimize=N)` (PyInstaller 6 hỗ trợ tham số
`optimize`). KHÔNG bật mù; chỉ sau khi C1.2/C1.3 xong.

### C2 — Thu gọn artifact (excludes)
`Sky-Player.spec:58` `excludes=[]`. Thêm excludes cho thứ chắc chắn không cần runtime để giảm size và
bề mặt: các dev-only (`pytest`, `pyright`, `ruff`, `soundcard`, `pyinstaller`) — *nếu* `collect_all`
vô tình kéo vào (kiểm tra `build/.../Analysis*.toc` hoặc warn log). Đừng exclude mù `tkinter`/`numpy`
nếu chưa xác nhận không dùng. Kiểm chứng: build trước/sau, so kích thước `dist/` và smoke test vẫn pass.

### C3 — Đồng bộ pin Python & version
- `pyproject.toml:6` `requires-python = ">=3.11,<3.15"` trong khi `.python-version=3.14` và toàn bộ
  tối ưu nhắm 3.14. Quyết định (ghi vào Decisions): GIỮ trần rộng cho forker, hay nâng sàn lên `>=3.13`
  (mốc `sys._is_gil_enabled` xuất hiện — để §A1 không cần nhánh getattr)? Khuyến nghị: **giữ rộng**
  (getattr ở A1 đã lo tương thích), trừ khi có lý do khác.
- Xác nhận `[project].version` (2.2.2) là nguồn version cho `build_app.py:get_project_version` →
  `windows_version_info.txt`. Không đổi version trong plan này trừ khi user yêu cầu.

### C4 — Build verification gate
- Chạy `uv run python -m build_app` (hoặc entry `build-app`) sạch trên Windows; smoke test
  `--selftest-textual` phải pass (`build_app.py:96-121`).
- Sau khi áp C1/C2: build lại, so size `dist/Sky-Player-v<ver>/`, xác nhận exe chạy + một lần playback
  thật vẫn cho telemetry sạch (sender_clean=true, không drop). KHÔNG cần đo lại latency (đã chốt).
- UPX/strip: **giữ tắt** (spec:75,77,93,94 — UPX hay gây false-positive AV; ghi rõ trong tuning-presets
  rằng forker muốn nhỏ hơn có thể tự bật, tự chịu rủi ro AV).

---

## 4. Thứ tự thực thi đề xuất

1. Pillar A (A1→A4) — độc lập, rủi ro thấp, có test. Làm trước.
2. Pillar B — tài liệu thuần, không rủi ro code. Làm song song.
3. Pillar C1 (audit assert) — *điều tra trước*, ra quyết định, rồi mới C2/C3, cuối cùng C4 build.

Mỗi pillar là một commit/PR riêng, diff nhỏ, dễ review (AGENTS.md change discipline).

---

## 5. Out of scope (đừng làm trong plan này)

- Bất kỳ thay đổi nào ở send-path/timing (lead/floor/spin/coordinator/onset).
- Free-threaded *build artifact* (`python3.14t` riêng): chỉ ghi chú trong tuning-presets như lựa chọn
  forker; KHÔNG dựng pipeline build thứ hai ở đây.
- Sub-interpreters (PEP 734) tách UI/dispatch — rewrite lớn, để plan khác.
- Game-acceptance ground truth (audio loopback) — hướng riêng, không thuộc vệ sinh/build.

---

## 6. Acceptance gates (Claude nghiệm thu)

- **G-A1** Trên môi trường giả lập free-threaded (monkeypatch `_is_gil_enabled→False`),
  `RealtimeProcessScope` KHÔNG gọi `setswitchinterval`, KHÔNG revert; trên GIL=True hành vi y cũ.
  Test mới trong `tests/test_realtime_scope.py` pass.
- **G-A2** Telemetry `runtime_options` có `gil_enabled` đúng; `switch_interval_tuning` vẫn phản ánh cờ.
- **G-A3** `--doctor` in dòng "GIL State" đúng cho build hiện tại.
- **G-B** `docs/tuning-presets.md` tồn tại; mọi cờ trong bảng khớp argparse thật (`main.py`); README trỏ tới.
- **G-C1** Có mục "Decisions" liệt kê từng assert (b) và xử lý; mức `--optimize` đã chọn có lý do.
- **G-C4** `uv run python -m build_app` pass smoke test; exe chạy + 1 playback thật telemetry `sender_clean=true`,
  0 drop; size `dist/` ghi lại trước/sau C2.
- **G-regress** `uv run pytest`, `uv run ruff check .`, `uv run pyright` đều sạch.

---

## 7. Decisions (điền khi thực thi)

- [x] `--optimize` level đã chọn: **`optimize=1`** — lý do: Audit C1 cho thấy hai nhóm assert:
  - **(a) Invariant debug thuần** (an toàn strip): không có — tất cả assert đều mang ngữ nghĩa.
  - **(b) Guard gánh nặng** (phải giữ):
    - `inputs.py:444` — `send_scan_code_batch_trusted`: guard trùng scan code trên hot-path.
    - `telemetry.py:250-257` — guard field bắt buộc trong TelemetryLogger.
    - `console_playback.py:570` — `assert isinstance(renderer, SnapshotRenderer)`: guard kiểu trước khi gọi Textual UI.
  - **Kết luận**: `--optimize 1` là mức an toàn nhất — giữ toàn bộ assert, chỉ bỏ docstring và `__debug__`-only block. `-OO` bị loại vì sẽ strip các guard (b) mà không có lợi đáng kể.

- [x] Asserts loại (b) và cách xử lý: **Giữ nguyên dạng `assert`**. Với `optimize=1`, chúng được bảo toàn. Nếu tương lai muốn `-OO`, cần chuyển chúng thành `if not cond: raise ValueError(...)` — nhưng không cần trong plan này.

- [x] `requires-python` giữ/nâng: **Giữ `>=3.11,<3.15`** — `getattr(sys, "_is_gil_enabled", None)` trong `_gil_enabled()` đã lo tương thích ngược tới 3.11. Không có lý do kỹ thuật để nâng sàn.

- [x] Excludes đã thêm: `pyinstaller, pyright, ruff, soundcard, pytest, _pytest` (tất cả là dev-only, không dùng ở runtime) — size dist trước/sau: **chưa đo** (cần chạy C4 build gate để ghi số cụ thể).

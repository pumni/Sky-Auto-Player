# Kế hoạch CHẨN ĐOÁN — Vì sao tăng hold làm lệch nhịp (timbre vs rhythm)

> **Bản 1/2.** Bản này KHÔNG sửa hành vi sản phẩm. Mục tiêu duy nhất: **đo để kết luận chính xác nguyên nhân** của hiện tượng "hold = 1 frame thì chuẩn nhịp nhưng sắc; tăng hold lên một chút thì âm sắc hay hơn nhưng xuất hiện lệch nhịp nhẹ; hold = 2 frame thì tệ".
> **Đối tượng thực thi:** AI coding agent.
> **Sau khi xong:** đọc kết luận và chuyển sang `docs/hold-timbre-rhythm-refactor-plan-2026-06-26.md` (bản 2) theo đúng nhánh mà bản này chốt.
> **Trạng thái mã:** branch `main` sạch. Mọi tham chiếu dòng phải đọc lại file trước khi dùng.

---

## 0. Bắt buộc đọc trước

### 0.1. Bối cảnh đã chốt (KHÔNG đo lại các điều này)
- **Onset ở mức schedule là bất biến giữa các profile.** `build_key_actions` đặt `down_at_us` không phụ thuộc `hold`/`min_hold` (xem `scheduler.py` Stage 2). Hai profile khác nhau chỉ ở `up_at_us = down_at_us + hold` và ngưỡng same-key. → Mọi khác biệt nhịp **không phải** do dời onset trong scheduler.
- **Cơ chế same-key up-gap gần như đã bị bác bỏ trên nhạc thật.** `docs/timing-principles.md` §5.2: same-key interval nhỏ nhất toàn corpus = **76 ms**, P50 ~996 ms; §6: `release_gap_us`/`repeat_release_gap_us` đã từng được hiện thực **và bị gỡ** vì "không bind trên nhạc thật". Vì vậy giả thuyết tái-kích same-key chỉ còn sống nếu **bài cụ thể của người dùng** có same-key lặp nhanh mà corpus không đại diện → Phase 0 kiểm tra điều đó.

### 0.2. Ba giả thuyết cần phân định
| Mã | Giả thuyết | Dấu hiệu đo được | Đo được bằng harness tổng hợp? |
|---|---|---|---|
| **H-CONTENTION** | Dispatch đơn luồng ưu tiên `up` trước `down`; hold dài đẩy các `up` lại gần onset phím khác → `up` chen trước, đẩy `down` trễ ở runtime | `lateness_us` của **down** tăng đơn điệu theo hold | **CÓ** (đây là trục SendInput-side) |
| **H-DURATION** | Lượng tử hóa frame phía game làm nốt non-integer-frame lúc dài 1 lúc 2 frame → trọng tâm cảm nhận xê dịch | `lateness_us` của down **phẳng** theo hold; chỉ độ dài/offset đổi | **KHÔNG** — cần đo audio trong game thật |
| **H-SAMEKEY** | Khe nhả same-key tụt < 1 poll trên chính bài người dùng | `min_same_key_up_gap_us` < `frame_us` ở hold đang dùng | **CÓ** (mức schedule, tĩnh) |

> **Nguyên tắc đánh giá (timing-principles §0):** observed game audio > deterministic measurement > code > docs. Harness tổng hợp ở đây là **rank 2**: nó có thể **xác nhận hoặc loại trừ H-CONTENTION và H-SAMEKEY** một cách dứt khoát. Nó **không** chứng minh được H-DURATION (rank 1, cần audio thật). Do đó logic kết luận là: *loại trừ contention + same-key ⇒ nguyên nhân còn lại là phía game/cảm nhận.*

### 0.3. Lệnh kiểm thử nền (chạy trước để xác nhận xanh)
```
uv run ruff check . && uv run pyright && uv run pytest
```
Ghi lại số test pass làm baseline. Bản này chỉ THÊM script + test đo, không sửa `src/` sản phẩm → baseline phải giữ nguyên.

---

## Phase 0 — Kiểm tra H-SAMEKEY trên chính bài của người dùng

**Mục tiêu:** xác định same-key up-gap có thực sự bind trên kho bài thật không. Nếu không, loại H-SAMEKEY khỏi vòng phân tích.

Tạo `scripts/audit_same_key_gap.py`:

```python
"""Audit same-key interval & up-gap across the song corpus, swept over hold.

Kết luận: nếu min(min_same_key_up_gap_us) >= frame_us ở MỌI mức hold thử nghiệm,
thì H-SAMEKEY KHÔNG bind -> loại khỏi phân tích (khớp timing-principles §5.2/§6).
"""
from __future__ import annotations

import math
from pathlib import Path

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.layouts import SKY_15_KEY_PROFILE

FPS = 60
FRAME_US = math.ceil(1_000_000 / FPS)
HOLD_FRAMES_SWEEP = [1.0, 1.25, 1.5, 2.0]


def policy_for(hold_frames: float) -> FrameTimingPolicy:
    # Decouple hold from min_hold: hold tăng, min_hold cố định 1 frame.
    base = TimingPolicy.from_dict({
        "min_hold_frames": 1.0,
        "min_hold_unframed_us": 22_000,
        "hold_frames": hold_frames,
    })
    return FrameTimingPolicy.from_timing_policy(base, fps=FPS)


def main() -> None:
    songs = sorted(Path("songs").glob("*.json"))
    if not songs:
        print("No songs found in songs/")
        return
    print(f"FPS={FPS} frame_us={FRAME_US}  (H-SAMEKEY binds khi up_gap < frame_us)")
    for hold_frames in HOLD_FRAMES_SWEEP:
        policy = policy_for(hold_frames)
        worst_gap = None
        worst_song = None
        worst_interval = None
        for sp in songs:
            try:
                song = parse_song_file(sp, SKY_15_KEY_PROFILE)
                res = build_key_actions(song, policy=policy)
            except Exception as e:  # bài lỗi parse — bỏ qua, ghi chú
                print(f"  [skip] {sp.name}: {e}")
                continue
            gap = res.min_same_key_up_gap_us
            if gap is not None and (worst_gap is None or gap < worst_gap):
                worst_gap = gap
                worst_song = sp.name
                worst_interval = res.shortest_same_key_interval_us
        binds = worst_gap is not None and worst_gap < FRAME_US
        print(
            f"hold={hold_frames:>4}f hold_us={int(policy.hold_us):>6} "
            f"min_up_gap={worst_gap} (song={worst_song}, "
            f"shortest_interval={worst_interval})  H-SAMEKEY_binds={binds}"
        )


if __name__ == "__main__":
    main()
```

Chạy:
```
uv run python scripts/audit_same_key_gap.py
```

**Cổng quyết định Phase 0:**
- Nếu `H-SAMEKEY_binds=False` ở mọi mức hold (kỳ vọng, khớp corpus) → **loại H-SAMEKEY**. Ghi vào findings.
- Nếu `H-SAMEKEY_binds=True` ở mức hold nào đó → H-SAMEKEY còn sống cho bài này → đánh dấu để bản 2 cân nhắc nhánh C.

---

## Phase 1 — Harness đo lateness của onset theo hold (phân định H-CONTENTION)

**Mục tiêu:** chạy lại đúng đường dispatch thật (`PlaybackEngine` + dispatch thread + adaptive lead) với backend mô phỏng độ trễ SendInput, **quét hold** trong khi **giữ min_hold = 1 frame**, rồi xem `lateness_us` của các sự kiện `down` có tăng theo hold không.

Tạo `scripts/measure_hold_rhythm.py`:

```python
"""Sweep hold (min_hold cố định 1 frame) và đo phân bố lateness của ONSET (down).

H-CONTENTION đúng  <=> p95/p99 lateness của down tăng đơn điệu theo hold.
H-CONTENTION sai   <=> lateness của down phẳng theo hold (=> nguyên nhân ở phía game).

Backend mô phỏng độ trễ SendInput theo phân bố telemetry thật (p50~477us, p99~953us)
để đường đơn luồng có chi phí gửi giống thực tế. KHÔNG phụ thuộc game/window.
"""
from __future__ import annotations

import math
import random
import statistics
import sys
from pathlib import Path

from sky_music.domain.parser import parse_song_file
from sky_music.domain.scheduler import build_key_actions
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.infrastructure.backend import _TrackedKeyState, ReleaseAllOutcome, BackendHealth
from sky_music.infrastructure.timing import PerfCounterClock, RealSleeper, SleepPolicy
from sky_music.layouts import SKY_15_KEY_PROFILE
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.playback_supervisor import PLAYBACK_FINISHED

FPS = 60
FRAME_US = math.ceil(1_000_000 / FPS)
HOLD_FRAMES_SWEEP = [1.0, 1.25, 1.5, 2.0]
TRUNCATE_US = 30_000_000  # 30s đầu mỗi bài cho nhanh; tăng nếu cần độ phủ.
SEED = 12345  # cố định để các lần chạy so sánh được


class SyntheticLatencyBackend(_TrackedKeyState):
    __slots__ = ("clock", "rng")

    def __init__(self, clock: PerfCounterClock, rng: random.Random) -> None:
        super().__init__()
        self.clock = clock
        self.rng = rng

    def get_health(self) -> BackendHealth:
        return BackendHealth(
            active_count=len(self.active_keys),
            possibly_active_count=len(self.possibly_active_keys),
            failed_release_count=len(self.failed_release_keys),
            last_error=self.last_error,
        )

    def _emit(self, scan_codes: tuple[int, ...], *, key_up: bool) -> int | None:
        r = self.rng.random()
        if r < 0.50:
            duration_us = 477
        elif r < 0.99:
            duration_us = int(self.rng.uniform(477, 953))
        elif r < 0.999:
            duration_us = int(self.rng.uniform(953, 1300))
        else:
            duration_us = int(self.rng.uniform(1300, 1695))
        t0 = self.clock.now_us()
        while self.clock.now_us() - t0 < duration_us:
            pass
        return self.clock.now_us()

    def release_all(self) -> ReleaseAllOutcome:
        to_release = self.active_keys | self.possibly_active_keys | self.failed_release_keys
        release_tuple = tuple(sorted(to_release))
        self.active_keys.clear()
        self.possibly_active_keys.clear()
        self.failed_release_keys.clear()
        return ReleaseAllOutcome(
            attempted=release_tuple,
            released_successfully=True,
            stuck_keys=(),
            verification_inconclusive=False,
        )


def policy_for(hold_frames: float) -> FrameTimingPolicy:
    base = TimingPolicy.from_dict({
        "min_hold_frames": 1.0,
        "min_hold_unframed_us": 22_000,
        "hold_frames": hold_frames,
    })
    return FrameTimingPolicy.from_timing_policy(base, fps=FPS)


def pct(values: list[int], p: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    return float(s[int(round(p * (len(s) - 1)))])


def run_one(song_path: Path, hold_frames: float) -> dict[str, float]:
    rng = random.Random(SEED)
    policy = policy_for(hold_frames)
    song = parse_song_file(song_path, SKY_15_KEY_PROFILE)
    sched = build_key_actions(song, policy=policy)
    actions = tuple(a for a in sched.actions if int(a.at_us) <= TRUNCATE_US)

    clock = PerfCounterClock()
    backend = SyntheticLatencyBackend(clock, rng)
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        telemetry_enabled=True,
        require_focus=False,
        clock=clock,
        sleeper=RealSleeper(),
        sleep_policy=SleepPolicy(),
        use_dispatch_thread=True,
        enable_adaptive_lead=True,
    )
    res = engine.play()
    if res != PLAYBACK_FINISHED:
        raise RuntimeError(f"{song_path.name}: playback code {res}")

    recs = engine.telemetry.records
    down_lat = [
        int(r.lateness_us)
        for r in recs
        if r.kind == "down" and r.runtime_outcome != "deferred_release"
    ]
    # Hold thực đo được (down->up cùng scan code) để kiểm tra biến thiên độ dài nốt.
    return {
        "down_n": len(down_lat),
        "down_lat_p50": pct(down_lat, 0.50),
        "down_lat_p95": pct(down_lat, 0.95),
        "down_lat_p99": pct(down_lat, 0.99),
        "down_lat_max": float(max(down_lat, default=0)),
        "down_lat_std": float(statistics.pstdev(down_lat)) if len(down_lat) > 1 else 0.0,
    }


def main() -> None:
    songs = [Path(p) for p in sys.argv[1:]]
    if not songs:
        songs = sorted(Path("songs").glob("*.json"))[:5]  # mặc định 5 bài đầu
    print(f"FPS={FPS} frame_us={FRAME_US}  TRUNCATE={TRUNCATE_US/1e6:.0f}s  seed={SEED}")
    print("Diễn giải: p95/p99 down-lateness TĂNG theo hold => H-CONTENTION đúng.\n")
    for sp in songs:
        print(f"== {sp.name} ==")
        header = f"{'hold(f)':>8} {'down_n':>7} {'p50':>8} {'p95':>8} {'p99':>8} {'max':>8} {'std':>8}"
        print(header)
        for hf in HOLD_FRAMES_SWEEP:
            try:
                s = run_one(sp, hf)
            except Exception as e:
                print(f"  [skip] hold={hf}: {e}")
                continue
            print(
                f"{hf:>8} {int(s['down_n']):>7} {s['down_lat_p50']:>8.0f} "
                f"{s['down_lat_p95']:>8.0f} {s['down_lat_p99']:>8.0f} "
                f"{s['down_lat_max']:>8.0f} {s['down_lat_std']:>8.0f}"
            )
        print()


if __name__ == "__main__":
    main()
```

Chạy trên một vài bài đại diện (ưu tiên bài người dùng thấy lệch rõ nhất — truyền đường dẫn để chỉ định):
```
uv run python scripts/measure_hold_rhythm.py "songs/<bai-1>.json" "songs/<bai-2>.json"
```
Hoặc mặc định 5 bài đầu:
```
uv run python scripts/measure_hold_rhythm.py
```

**Lưu ý độ tin cậy:**
- Chạy mỗi cấu hình ≥ 3 lần (đổi `SEED` hoặc bọc vòng lặp) để loại nhiễu OS scheduler; lấy trung vị các lần. Số liệu down-lateness biến động vài chục–trăm µs là bình thường; tín hiệu cần tìm là **xu hướng đơn điệu theo hold**, không phải một con số đơn lẻ.
- Đóng ứng dụng nặng khác khi đo (jitter nền làm nhoè kết quả).
- Backend mô phỏng dùng cùng phân bố telemetry thật → chi phí gửi giống thực; nhưng đây vẫn là rank-2, không thay cho audio thật.

---

## Phase 2 — Bài kiểm tra contention có kiểm soát (làm rõ tín hiệu Phase 1)

**Mục tiêu:** nếu Phase 1 mơ hồ (xu hướng yếu), dựng hai bài tổng hợp để khuếch đại/triệt tiêu contention, cô lập cơ chế.

Thêm vào `scripts/measure_hold_rhythm.py` (hoặc file riêng `scripts/measure_hold_rhythm_synth.py`) hai timeline:
- **DENSE-ALT:** chuỗi nốt **khác phím** xen kẽ, IOI ≈ `hold` (ví dụ 17–20ms) để release của nốt N rơi sát onset nốt N+1 → tối đa hoá contention.
- **SPARSE:** cùng số nốt nhưng IOI ≥ 200ms → release không bao giờ gần onset → contention ≈ 0.

Dựng `Song` trực tiếp:
```python
from sky_music.domain import Song, Note, NoteKey, Millis

def dense_alt(n=400, ioi_ms=18):
    keys = ["Key0", "Key1", "Key2", "Key3", "Key4"]
    notes = tuple(
        Note(time_ms=Millis(i * ioi_ms), key=NoteKey(keys[i % len(keys)]))
        for i in range(n)
    )
    return Song(name="DENSE-ALT", notes=notes)

def sparse(n=400, ioi_ms=200):
    keys = ["Key0", "Key1", "Key2", "Key3", "Key4"]
    notes = tuple(
        Note(time_ms=Millis(i * ioi_ms), key=NoteKey(keys[i % len(keys)]))
        for i in range(n)
    )
    return Song(name="SPARSE", notes=notes)
```
Chạy `run_one`-tương đương cho cả hai, quét hold.

**Diễn giải:**
- Nếu **DENSE-ALT** có p95/p99 down-lateness tăng mạnh theo hold nhưng **SPARSE** thì phẳng → **H-CONTENTION xác nhận** (contention chỉ xuất hiện khi release sát onset).
- Nếu cả hai đều phẳng → contention **không phải** nguyên nhân → còn lại H-DURATION/game-side.

---

## Phase 3 — Tổng hợp kết luận

Tạo `docs/hold-timbre-rhythm-findings.md` và điền theo mẫu (đây là output bắt buộc — bản 2 đọc file này để chọn nhánh):

```markdown
# Findings — hold timbre vs rhythm (ngày chạy: ____)

## Cấu hình đo
- FPS, số bài, số lần chạy/cfg, máy đo.

## Phase 0 (H-SAMEKEY)
- Bảng min_up_gap theo hold. Kết luận binds = True/False.

## Phase 1 (H-CONTENTION trên nhạc thật)
- Bảng down-lateness p50/p95/p99/max/std theo hold, cho từng bài.
- Xu hướng: down-lateness có tăng đơn điệu theo hold không? (Có/Không/Không rõ)

## Phase 2 (contention kiểm soát)
- DENSE-ALT vs SPARSE. Có phân ly không?

## VERDICT (chọn đúng MỘT)
- [ ] A. H-CONTENTION xác nhận  -> bản 2, Nhánh A
- [ ] B. Contention bị loại; nguyên nhân ở game/cảm nhận -> bản 2, Nhánh B
- [ ] C. H-SAMEKEY bind trên bài người dùng -> bản 2, Nhánh C
- (Có thể A+C đồng thời nếu cả hai cùng đúng.)

## Số liệu thô đính kèm
- Dán bảng stdout hoặc đường dẫn logs/.
```

**Tiêu chí "kết luận đủ mạnh":**
- A khẳng định khi: ở ≥ 2/3 bài thật, `down_lat_p99(hold=2.0) − down_lat_p99(hold=1.0) ≥ 1× frame_us (~16.7ms)` **hoặc** ≥ 50% theo giá trị; và Phase 2 cho phân ly DENSE/SPARSE rõ.
- B khẳng định khi: chênh down-lateness giữa hold=1.0 và hold=2.0 < ~½ frame ở mọi bài **và** DENSE/SPARSE đều phẳng. (Lúc đó cần bước đo audio thật ở bản 2, không phải sửa code mù.)
- C khẳng định khi Phase 0 cho `binds=True`.

---

## Quy tắc
1. Không sửa `src/` sản phẩm trong bản này (chỉ thêm `scripts/` + `docs/findings`). Baseline `uv run ruff check . && uv run pyright && uv run pytest` phải giữ xanh.
2. Không "đoán" verdict — chỉ chốt theo số. Nếu không rõ, ghi "Không rõ" và đề xuất tăng số bài/số lần chạy.
3. Không retry mù khi script lỗi: đọc lỗi (thường là tên key bài không map được scan code), bỏ bài đó và ghi chú.

## Cam kết hoàn thành
- [ ] `scripts/audit_same_key_gap.py` chạy được, có bảng.
- [ ] `scripts/measure_hold_rhythm.py` chạy được trên ≥ 3 bài thật, ≥ 3 lần/cfg.
- [ ] Phase 2 chạy nếu Phase 1 mơ hồ.
- [ ] `docs/hold-timbre-rhythm-findings.md` có VERDICT chọn đúng một nhánh.

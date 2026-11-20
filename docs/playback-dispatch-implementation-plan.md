# Kế hoạch triển khai đường gửi phím Sky Auto Player

| Mục | Giá trị |
| --- | --- |
| Nhánh triển khai | `refactor/playback-dispatch-hot-path` |
| Base để review thay đổi | `a425ba017d2455952c758133068284dc6de89aa3` |
| Nền tảng | Windows 11 x64 MSVC, Rust và Cargo 1.99.1, Cargo.lock hiện hành |
| Điều phối | Một draft PR vào `main`, một nhánh từ đầu đến cuối, review theo exact HEAD |
| Trạng thái ban đầu | Gói 1–3 được giao trong phạm vi dưới đây. Benchmark vật lý và thay đổi chính sách do Leader/owner quyết định. |

Mục tiêu là giảm công việc giữa lúc packet đủ điều kiện và lúc gọi `SendInput`, giữ vòng đời một sender với nhả phím có thể kiểm chứng. [Architecture](architecture.md), [playback engine](playback-engine.md), [verification](playback-verification.md) và [security](../SECURITY.md) là các hợp đồng đang áp dụng. Kế hoạch này chốt phạm vi triển khai của PR.

## 1. Vai trò và quy trình GitHub

- **Leader** sở hữu kiến trúc, các invariant, phạm vi, thứ tự gói, quyết định thay đổi chính sách, review và kết luận nghiệm thu. Quyết định mới được ghi trong PR. Chủ repo giữ quyền quyết định merge và cấp phép test vật lý.
- **Implementation agent** sở hữu triển khai và xác minh theo các gói dưới đây. Được tự chọn tên biến, vị trí helper, tổ chức commit và cách tái sử dụng test seam trong phạm vi đã chốt. Đưa mâu thuẫn contract, thay đổi bảo đảm an toàn, wire schema hoặc dữ liệu thiếu lên PR kèm code path và bằng chứng. Tiếp tục phần không phụ thuộc trong lúc chờ quyết định.
- Fetch nhánh có sẵn và checkout trên máy Windows. Tất cả commit và fixup được push fast-forward vào nhánh này. Dùng detached worktree nếu cần đọc baseline hoặc build hai binary. Không tạo thêm nhánh công việc, rewite lịch sử đã push, force-push, mở PR phụ hay merge main vào nhánh trong lúc review nếu Leader chưa yêu cầu.
- Mỗi gói là một lát cắt hoàn chỉnh xuyên scheduler, adapter, lifecycle, tests và contract khi có liên quan. Có thể chia commit trong gói theo ý nghĩa. Push khi lát cắt sẵn sàng review và ghi comment bàn giao trên draft PR. Có thể tiếp tục phần gói sau đã được giao sau khi đã bàn giao SHA. CI đang chạy hoặc đỏ không trì hoãn review. Check bắt buộc của repo vẫn áp dụng cho quyết định merge.
- CI, source review, local gates và benchmark có trạng thái độc lập. Lỗi baseline phải được chứng minh bằng cùng command trên baseline sạch trước khi gọi là pre-existing. Leader review theo SHA, để lại finding có mức độ, file/line và kết quả cần sửa. Author sửa trên cùng nhánh và ghi SHA sửa trong thread liên quan.

Mẫu bàn giao cho mỗi gói hoặc đợt sửa:

```text
Gói: 1 / 2 / 3 hoặc Fix follow-up
Base sản phẩm / SHA đầu gói / SHA HEAD / Git tree:
Commits và phần thay đổi:
Đường cũ đã xóa:
Symbol hoặc test seam giữ lại và caller sử dụng:
Windows runner / rustc -Vv / cargo -V:
Command, exit code, số pass/ignored/skipped:
CI URL và trạng thái nếu có:
Evidence URL hoặc SHA-256 / experiment classification:
Blocker hoặc quyết định cần Leader:
```

Chỉ đăng dữ liệu và log đã loại bỏ thông tin máy riêng, token và đường dẫn nhạy cảm. Dùng comment cho tóm tắt và chỉ định SHA, artifact của job cho raw evidence nếu có. Không đưa file build, raw trace hoặc bản sao báo cáo vào Git. Leader review phát hiện từ GitHub theo SHA kể cả khi không có artifact tùy chọn.

## 2. Các quyết định đã chốt

| Chủ đề | Hợp đồng triển khai |
| --- | --- |
| Đường gameplay | Một worker OS là writer vật lý duy nhất của phiên. Một đường packet production gọi Win32 `SendInput` bằng scan code của profile hiện có. Up đứng trước Down trong gói Mixed, các mask rời nhau, tối đa 15 sự kiện. Dry-run không chạm backend vật lý. |
| Quyền gửi | Mọi packet nhạc cần target stamp và release owner thuộc cùng phiên. Kiểm tra retained process liveness và foreground HWND/PID tươi, sau đó xác nhận control/generation sát call. WinEvent, cờ focus và process callback chỉ có thể báo thức hoặc phủ quyết. Không dùng kết quả kiểm tra từ trước spin để cấp quyền gửi sau spin. |
| Modifier và lease | Packet có Down giữ scan các modifier hiện hành, phân biệt Held và Inconclusive theo API hiện có. Tạm giữ chính sách kiểm tra tuổi lease của Up-only đang chạy tại base và hành vi expiry→terminal. Bất nhất giữa mã và tài liệu về Up-only lease phải được sửa mô tả trong contract, không sửa chính sách bằng suy diễn. |
| Scheduler | QPC tuyệt đối. Giữ floor H/G dựa trên successful completion, resume barrier, một deadline độc lập cho Up tới hạn, không rebase epoch chỉ vì modifier. Giữ thứ tự và hành vi của packet đã trễ theo contract hiện có. Một syscall đã thử cho onset thì onset đó không được phát lại. |
| Journal và cleanup | Ghi journal trước syscall với đúng effective mask. Commit lịch chỉ khi transport đầy đủ và QPC hợp lệ. Raw zero, partial, contradictory count, thiếu QPC trước/sau call và panic phân biệt theo contract. `needs_up` cần transport, `needs_probe` chỉ cần probe. Giữ giới hạn cleanup hiện hành, quyền gửi terminal background duy nhất khi đã được policy hiện tại cho phép, và retained origin. |
| Lifecycle | Giữ Stop, Skip, Panic, suspend, supervisor lease, focus pause và resume semantics. Giữ supervisor lifetime guard và watchdog. Recovery chỉ tiếp quản sau join. Quarantine chặn phiên mới. Terminal UI publication đi sau retirement commit theo session ID. |
| Hiệu năng và phụ thuộc | Giữ timer high-resolution hiện có, MMCSS/thread guards, affinity mặc định, process QoS Default, startup calibration và spin policy production ở base trong PR này. Giữ deps, crate layout và wire schema đang dùng. Dữ liệu mới cho acceptance ở test/measurement seam có caller cụ thể. Không thêm alternate sender hoặc production fallback song song. |

Nếu phát hiện invariant production đã sai tại base, lập case có thể lặp lại, ghi đường mã và đề xuất lên PR. Leader quyết định phạm vi sửa trước khi CLI thay đổi hành vi.

## 3. Gói 1 — Hợp nhất đường gửi và bỏ dựng lại Mixed

Thực hiện từ eligibility tới commit trong một gói. Entry points chính là [dispatch loop](../rust/crates/sky_player/src/engine/worker/dispatch_loop.rs), [scheduler staging](../rust/crates/sky_player/src/engine/worker/dispatch_loop_scheduler.rs), [admission](../rust/crates/sky_player/src/engine/worker/admission.rs), [prepared packet](../rust/crates/sky_dispatch_win32/src/input/packet.rs) và [tracked sender](../rust/crates/sky_dispatch_win32/src/input/tracked/packet_send.rs).

1. **Stage một lần.** Chuẩn bị packet kind, bitmask, metadata QPC và target stamp. Tính Up-only timing window từ Up slots trước wait, đồng thời bảo toàn định danh source và dữ liệu attribution của Mixed. Dời kiểm tra cấu trúc bất biến ra khỏi hậu deadline. Lấy hoặc xác minh release-origin của các slot trước wait nếu an toàn với quyền sở hữu hiện tại, và luôn giữ xác nhận currency tại sender. Không kéo thao tác OS tươi về trước deadline chỉ để làm giảm con số hậu deadline.
2. **Adapter trả view Up-only hợp lệ từ cùng packet.** Dùng `PreparedPhysicalPacket` và `PreparedPacketView` đang có. Thu hẹp view theo prefix đã khởi tạo và effective mask `(up_mask, 0)`. Từ chối prefix rỗng, giữ dữ liệu hoàn chỉnh cho Full view. Đặt lifetime/unsafe trong adapter. Bên worker không tạo raw pointer, không tự dựng `INPUT` thứ hai và không truyền số phần tử rời rạc khỏi packet. Giữ helper sender theo deadline có caller thật trong native calibration.
3. **Một lần chọn, một đường gửi.** Khi Down bị chặn trước syscall và Up đủ điều kiện, dùng cùng candidate với effective Up-only view và Up-only timing, giữ Down để replan. Nếu chặn do target/focus/terminal control hoặc lỗi không cho phép gửi, đi qua lifecycle tương ứng. Xử lý cả nhánh replan trước wait và nhánh cuối sau wake, giữ deadline Up riêng và xử lý hủy trong lúc chờ. Xóa đường restage/recursive dispatch sau cutover. Dùng cùng admission flow để loại các lần đọc control kề nhau khi chúng không giữ race guard khác mốc. Giữ kiểm tra early/final khi chúng bảo vệ hai cửa sổ khác nhau.
4. **Từ final admission đến commit.** Sau khi chọn effective packet, dùng effective masks cho owner-proof, journal, raw count, completion commit, retry/replan và diagnostics. Ngay trước syscall, giữ các control/identity check cần thiết, ghi in-flight và lấy authoritative pre-call QPC. Sau syscall, lấy completion evidence và cập nhật journal/scheduler. Không ghi phantom Down khi chỉ gửi prefix. Không suy ra từng phím đã được ghi nhận từ một count partial nếu contract hiện hành giữ trạng thái uncertain. Không dùng nhánh safety Up để chạy retry nhạc.
5. **Dọn đường cũ cùng gói.** Xóa helper restage chỉ phục vụ Mixed đã thay, nhánh đệ quy, wrapper, tham số, module import và test seam mất caller. Chuyển tests sang đường mới hoặc xóa tests chỉ xác nhận hình dạng cũ, đồng thời giữ assertions bảo vệ hành vi. Kiểm tra mọi feature/target và cả `xtask` hoặc scripts phụ thuộc tên symbol. Đồng bộ hợp đồng playback và mức coverage của tests.

**Bàn giao Gói 1.** Coverage bằng test hiện có và test hồi quy cụ thể cho:

- Down-only, Up-only, Mixed, Mixed→Up prefix lúc pre-wait và lúc final gate, không heap alloc trên đường khỏe, 1/5/15 phím. Prefix đúng thứ tự, đúng số events, một call tối đa, không journal hoặc commit Down. Nếu chưa tới floor Up, không gửi sớm.
- Đổi focus, owner hoặc generation trong khi wait và trong final admission. Dừng/pause/skip/panic/suspend/lease hết hạn giữa stage và call. Inconclusive, multi-modifier, modifier hold timeout và các Up độc lập đến hạn.
- Raw zero, partial, full transport với thiếu QPC completion, lỗi QPC trước call, panic trước/sau journal. Assert đúng raw-call count và release obligations. Không tiêu thụ lại onset đã thử. Giữ H/G tại boundary, reset/resume và parity của authored schedule.

## 4. Gói 2 — Giảm chi phí quan sát và khóa phép đo

Thực hiện trên kiến trúc Gói 1. Xem [metrics](../rust/crates/sky_player/src/engine/telemetry/metrics.rs), [dispatch observation](../rust/crates/sky_player/src/engine/worker/dispatch/observation.rs), [native runbook](playback-verification.md) và [harness](../rust/crates/sky_player/src/bin/rt_native_acceptance.rs).

1. **Telemetry tùy cấu hình.** Giữ tick counters thuộc worker và kiểm tra cadence hiện tại trong tick domain. Chuyển đổi µs và dựng projection ra consumer ở phần làm được mà vẫn giữ startup/paused/terminal publication và schema. Dữ liệu được publish nguyên một snapshot nhất quán, không đọc lẫn generation. Giữ observer flags và accounting khi mất sample. Tháo projection cũ và số liệu trùng sau khi chuyển caller. Giữ cơ chế wake hiện hành trong PR này để không đánh đổi chi phí polling bằng một wake trên mỗi packet.
2. **Đo các ranh giới khác nhau.** Dùng acceptance/test seam hiện có để đối chiếu `authored`, `eligible`, kernel timer return, `gate_start`, `gate_end`, `pre_call` và `completion`. Công cụ đo chi tiết dùng riêng hoặc lấy mẫu thưa, ngoài production mặc định. Giữ distinction unavailable/zero, tổng mẫu và queue drops. Chạy diagnostics off/on riêng. Nếu không thể ghi nhận một timestamp không gây sai phép đo, báo unavailable và ghi cách kiểm chứng thay thế.
3. **Hiệu chuẩn.** Thêm hoặc sử dụng test tất định chứng minh P99 của 6 mẫu là max và một outlier có thể chạm trần. Kiểm tra startup calibration placement của baseline/candidate bằng seam đã có. PR này giữ ngưỡng và thứ tự production đã pin. Nếu có evidence, báo để Leader quyết định có đổi policy ở checkpoint sau. Không tạo option runtime hoặc bản sao timer chỉ để chạy thử.

**Bàn giao Gói 2.** Coverage cho diagnostics tắt, bật, snapshot không lấy được slot, paused/terminal publication, mất telemetry sample và schema tương thích. Nộp số mẫu và p50/p95/p99/max cho `wake → gate_start → gate_end → pre_call`, `pre_call → completion`, `eligible → pre_call`, và CPU/spin khi có phép đo hợp lệ. Tách phần trễ từ floor H/G ra khỏi sender lateness. So sánh không được suy gameplay từ sender metrics.

## 5. Gói 3 — Kết thúc phiên, đối soát và dọn toàn tuyến

1. **Một record của attempted anomaly.** Production kết thúc nhạc ở sender anomaly đầu tiên. Thay cấu trúc rolling 16 phần tử trong worker bằng một record first attempted anomaly có giới hạn, tái sử dụng formatter/xuất terminal hiện có. Giữ thứ tự nguyên nhân, secondary error và tên trường xuất mà caller đang dùng. Xóa buffer, counters, test seam và comments về việc tiếp tục sau lỗi nếu chúng chỉ phục vụ đường cũ. Trường hợp fail Closed phải đi tới drain, kể cả khi suspend đến sau cùng.
2. **Terminal metrics và release.** Soát hai terminal snapshots trong finalizer. Nếu audit và tests xác nhận snapshot trung gian không phải hợp đồng hoạt động, giữ một publication đủ dữ liệu sau guard teardown và trước terminal outcome. Nếu có caller đọc snapshot đó với ý nghĩa riêng, ghi bằng chứng vào PR để Leader quyết định. Giữ probe ngay, backoff hiện hành, cùng giới hạn raw calls cho cleanup. Probe cadence mới chỉ là một thí nghiệm đã định danh cần evidence và quyết định Leader.
3. **Ownership, retirement, và final sweep.** Xác nhận Stop/Skip/Panic/suspend/focus/lease/anomaly không chuyển quyền release trước join, residual giữ đúng origin và successor bị chặn khi còn quarantine. Giữ trạng thái paused journal sạch ở zero periodic wake. Rà touched crates và shell caller, các feature `test-support` và `real-input-acceptance`, tests, exports, config và docs. Gỡ đường cũ, code bị bypass, TODO chuyển tiếp, cờ dual-path và bản sao test seam phát sinh trong refactor. Giữ những đường có caller thực và ghi caller đó khi bàn giao. Không dọn rộng sang updater, UI, packaging hoặc CI.

**Bàn giao Gói 3.** Evidence cho one-winner terminal reason, behavior của cleanup bounded, raw-call count và `needs_up / needs_probe` cả khi QPC lỗi. Xác nhận join → retirement/quarantine → UI publish, UI event delivery failure và quyền recovery. Bàn giao danh sách các symbol/files bị xóa và lý do cho các compatibility path còn dùng. Đề xuất bỏ Markdown kế hoạch khỏi diff sản phẩm bằng commit riêng sau khi Leader đã chốt nghiệm thu. Bản kế hoạch ở commit ban đầu và PR vẫn truy được.

## 6. Kiểm tra Windows và tiêu chí review

Chạy từ repo root, dùng toolchain pin. Với từng gói chạy test liên quan rồi format/lint và `git diff --check` trước khi bàn giao. Gói 3 chạy bộ cuối cùng theo fencing README/runbook. Ghi lệnh thực tế và kết quả, không ghi pass cho lệnh chưa chạy.

```powershell
rustc -Vv
cargo -V
git diff --check
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo xtask check rust
cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc -- --test-threads=1
cargo test --locked --manifest-path rust/xtask/Cargo.toml audits::security::tests -- --test-threads=1
cargo check --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance
cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance -- -D warnings
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- --test-threads=1
cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- -D warnings
pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
cargo xtask check static --skip-supply-chain
cargo xtask check all
```

Chạy offline lifecycle evidence từ checkout sạch theo `scripts/run_dispatch_lifecycle_evidence.ps1`, với output ngoài repo. Ghi rõ `native_sendinput_calls=0`. Chạy qualification `PlanOnly` nếu có đúng manifest baseline/candidate; đối chiếu source, tree, lockfile, feature, build profile và binary hash. Kết quả build, mock, offline evidence và PlanOnly có nhãn riêng.

**Native ReceiveOnly.** Chỉ khi owner đã phê duyệt chính xác máy Windows, foreground receiver sở hữu bởi project và phạm vi input theo [runbook](playback-verification.md). Trước đó thực hiện các check độc lập và ghi `Native ReceiveOnly: NOT RUN`, `Live Sky gameplay: NOT RUN`. Nếu có phê duyệt, reuse `PlaybackCoreLifecycle`: 6 cặp AB/BA × 15 giây, 5 workload × Quiet/Contention, margin và tám safety scenarios theo runbook. Sau đó có thể đối chiếu baseline chẩn đoán `a425ba0` nếu Leader giao. Baseline qualification bất biến `2544daa…` thiếu successor Armed QPC ở một phần chuyển tiếp, giữ `INCOMPLETE` đúng quy tắc hiện hành.

| Kiểm tra của Leader | Điều kiện đạt |
| --- | --- |
| **Kiến trúc** | Một sender production, một đường commit, prefix không restage/recursive dispatch, no heap allocation trên healthy path, mỗi symbol mới có caller theo đúng feature. |
| **Tính đúng đắn** | Không gửi sai nghĩa vụ/owner trong race cases, không phá H/G, không replay, không sai terminal reason hoặc gọi Up ngoài phạm vi journal. |
| **Hiệu năng** | Có so sánh lượng công việc và attribution trên cùng case. Native measurements nếu có nêu chính xác SHA, workload, sai số/missing và dữ liệu riêng cho sender/receiver. Leader quyết định khi có tail latency hoặc CPU trade-off; không tuyên bố tối ưu đã được xác nhận từ mock. |
| **Vòng đời** | Cleanup bounded, journal reconciliation giữ đúng semantics, successor fenced. Không còn code song song hoặc deadcode trong phạm vi refactor. |
| **Merge** | Review theo exact final SHA, findings đã xử lý, gates repo có kết quả, hạn chế đo đạc ghi rõ. Leader tổng hợp và owner quyết định merge. |

## 7. Lệnh bắt đầu cho implementation agent

```powershell
git fetch origin refactor/playback-dispatch-hot-path
git switch --track origin/refactor/playback-dispatch-hot-path
```

Nếu đã có nhánh local tracking, switch sang nhánh đó rồi cập nhật fast-forward. Đọc `AGENTS.md`, hợp đồng playback, cùng kế hoạch này. Bắt đầu Gói 1 end-to-end. Sau push, ghi bàn giao Gói 1 trong draft PR bằng exact SHA. Không cần đợi CI xanh để yêu cầu review.

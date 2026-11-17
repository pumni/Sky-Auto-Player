# Redesign vòng đời gửi phím và dọn mã cũ

| Mục | Giá trị |
| --- | --- |
| Repository | `pumni/Sky-Auto-Player` |
| Base đối chứng | `d0f65e7c412041a6d502f81f469872198db7adc5` |
| Base tree | `19aae9e41bc667a2f991001ed105b6ffdebb8bc6` |
| Nhánh triển khai | `refactor/dispatch-session-redesign` |
| Đích PR | `main` |
| Môi trường | Windows 11, `x86_64-pc-windows-msvc`, Rust/Cargo 1.99.1, Cargo.lock và Bun theo pin của repo |
| Điều phối | Một draft PR, review từng exact SHA. **M1 được giao ngay khi draft PR mở**. M2 và M3 bắt đầu khi Leader giao trên cùng PR. |

## 1. Mục tiêu và thẩm quyền

Đưa việc lập lịch, dựng gói và chẩn đoán ra khỏi đoạn sát syscall khi hợp đồng cho phép. Ưu tiên `eligible deadline → SendInput entry` và độ trễ Stop → trạng thái nhả được xác minh. Giữ H/G, thứ tự, nghĩa vụ nhả và cơ hội Sky nhận đủ hợp âm. Kết thúc đợt này với một đường phát vật lý chuẩn trong production, cùng các công cụ kiểm chứng cần thiết. Xóa mã chết phát sinh bởi redesign và mã cũ không còn caller trong phạm vi playback.

| Vai trò | Trách nhiệm |
| --- | --- |
| **Leader** | Chốt hành vi, kiến trúc, hợp đồng tương thích, phạm vi native, oracle và tiêu chí nghiệm thu. Review diff/test tại exact SHA, ghi phát hiện và giao mốc kế tiếp trên PR. |
| **Implementation agent CLI** | Triển khai trọn mốc được giao, gồm code, test, caller, tài liệu sở hữu hành vi và bằng chứng. Tự chọn tên nội bộ, cách chia commit, cách tái sử dụng code và test fixtures. Ghi báo cáo và push lên nhánh đã chỉ định. |
| **Chủ repo** | Quyết định merge và release sau khi có kết luận Leader và các kiểm tra merge đã đủ. |

Nếu phát hiện phải đổi hành vi playback, ranh giới an toàn, API/schema, khả năng tương thích với artifact cũ, nguồn chứng cứ, công cụ qualification, chính sách performance hoặc phạm vi mốc, ghi đề xuất trên PR kèm SHA/path, ví dụ tái hiện, các phương án và phần bị ảnh hưởng. Chờ quyết định cho phần phụ thuộc. Tiếp tục công việc độc lập trong mốc đã giao. Mỗi mốc là một lát cắt từ production entry point đến test/evidence. CLI có thể push nhiều commit có tính liên kết trong mốc. Lịch sử nhánh dùng fast-forward, PR dùng cho mọi mốc. Việc đổi base, force-push, chỉnh rules/workflow, merge, release hoặc giao mốc thuộc thẩm quyền Leader/chủ repo.

**Tham chiếu:** [AGENTS](../../AGENTS.md) · [SECURITY](../../SECURITY.md) · [Playback engine](../playback-engine.md) · [Playback verification](../playback-verification.md) · [CI topology](../ci-validation-topology.md) · [Checkpoint #666](https://github.com/pumni/Sky-Auto-Player/issues/666) · [PR đã merge #667](https://github.com/pumni/Sky-Auto-Player/pull/667)

## 2. Các quyết định đã chốt

### Ownership và control

- **Bind một target bất biến trước khi cho phép Arm.** Desktop và phiên production Rust có thể chốt token riêng của phiên chứa HWND, PID, process handle và identity đã xác minh. Hoàn tất lifetime và việc chia sẻ read-only lúc chuẩn bị. Runtime phát chỉ mượn hoặc sở hữu cấu trúc ổn định trong suốt phiên. Tránh giữ writer permit, chạm owner-authority mutex hoặc clone owner authority từng gói. Không duy trì một engine production cũ song song.
- **Target mới thuộc phiên mới.** Đổi HWND, mất owner liên tục hoặc kết thúc process dẫn tới terminal fail-closed khi được xác nhận. Không chuyển nghĩa vụ nhả sang cửa sổ khác, kể cả tái dùng cùng giá trị HWND. Desktop phải đợi worker quiescent và áp dụng chính sách release/quarantine hiện tại trước khi cấp quyền phát tiếp. Mất foreground với cùng target còn sống vẫn là Focus pause có thể phục hồi.
- **Worker là chủ trạng thái phát và journal.** Supervisor/power callback được yêu cầu hủy và đánh thức, giữ tiến độ lease, phát focus hint và đọc snapshot. Một focus hint positive chỉ là gợi ý. Giữ terminal latches, startup linearization, kiểm tra lease age và cơ chế watchdog lưu hành. Stop/Skip/Panic/Suspend đã được quan sát tại final gate chặn attempt kế tiếp. Nếu sự kiện đến sau lần quan sát cuối trong khi syscall đang vào/đang chạy, dựa trên journal và kết quả gửi để cleanup. Ack dừng và retirement phải đợi đúng ranh giới quiescence. Bảo toàn thứ tự Pause/Resume hiện hành, chống lost wake và chặn callback cũ tác động phiên mới.
- **Bằng chứng Win32 giữ nguyên về độ mạnh.** Mọi musical packet, kể cả Up-only, cần fresh foreground HWND, PID và continuity qua process handle. Down giữ preflight và modifier guard. Pause/resume giữ grace, preflight, origin proof và barrier. Supervisor poll cũ hoặc poll đến trễ không có quyền hủy phiên khác, đóng handle đang dùng hay đổi binding. M1 giữ truy vấn continuity hiện hành. Process waitable handle, thay đổi lease/watchdog hoặc giản lược truy vấn Win32 cần quyết định khác.
- **An toàn input theo hợp đồng repo.** Chỉ dùng `SendInput` cho gameplay, dùng Windows API quan sát đã được chấp nhận. Không dùng hooks, injection hay thao tác game. `SendInput` đưa sự kiện vào luồng input hệ thống. Kiểm tra foreground không khóa quyền định tuyến của Windows qua syscall. Origin check khi safety Up đang background xác minh nguồn nghĩa vụ theo policy hiện tại. Nó không chứng minh HWND nào nhận phím.

### Scheduling, vận chuyển và lỗi

- Giữ một executor 15 slot, một Down cursor, H/G theo completion QPC, Up độc lập, hợp âm Down nguyên vẹn, mask Up/Down rời nhau và Up trước Down. QPC tick, offset timeline và duration giữ kiểu phân biệt. Tối đa 15 `INPUT` trong một gói hợp lệ.
- Giữ `Due` gửi ứng viên đã stage rồi chọn lại ở vòng sau, `Interrupted` hủy ứng viên để xử lý điều khiển. Giữ musical Up-only fallback khi các điều kiện an toàn của Up vẫn đủ. Bản redesign này không thay đổi tempo, margin, per-note drop/retime, late-wake coalescing, MMCSS, spin guard, timer policy hay pre-roll mặc định.
- Tạo nghĩa vụ dự phòng trước FFI. Phân biệt pre-call rejection với post-call result. Một authored Down tối đa một syscall attempt. Chỉ commit healthy completion khi có chứng cứ đủ về count và QPC. Giữ narrow proven raw-zero Down-only, reconciliation khi short/mixed/unknown, terminal anomaly latch, các mask confirmed/possibly-active/in-flight, cleanup và quarantine. QPC được lấy sau cleanup có chứng cứ mới mở barrier `Q_cleanup + G`. Một benchmark nhanh hơn không cho phép bỏ chứng cứ này.
- Giữ wire DTO, schema 25, consumer đang hỗ trợ, các verdict native và chính sách lỗi UI transport hiện tại. Mã nội bộ được thu gọn. Trường tương thích còn hợp đồng có thể được dựng ở lớp snapshot/export với cùng giá trị và provenance. Giữ security, updater, signing, CI và toolchain ngoài phạm vi sửa.

## 3. M1 — Ownership và vòng đời end-to-end

**Trạng thái: giao ngay.** Chuyển đường Desktop production sang mô hình target bất biến, xuyên từ Prepare/Start, worker startup đến pause, Stop, terminal cleanup và retirement. Đọc các ranh giới hiện tại ở `sky_player/src/engine/{session,target,shared}.rs`, `worker/{admission,dispatch_loop,cleanup}.rs`, `sky_dispatch_win32/src/{focus,input/tracked}`, Desktop `native_runtime/playback/{session,retirement}.rs` và `power_lifecycle.rs`. Rà caller của set/bind/republish/poll/close trước khi thay chữ ký.

### Phải giao

1. Chốt target và truyền quyền dùng handle vào vòng đời native. Chỉ thu hồi authority sau khi đường có thể dùng nó đã dừng. Giữ OS poll tươi ở cổng cuối, kiểm tra preflight, modifier, lease và power theo quyết định ở mục 2.
2. Bỏ đường publish/rebind động của production, writer permit, các lượt xác thực stamp lặp chỉ để bảo vệ rebind, nhánh retry `PermitBusy` và field/counter riêng đã mất tác dụng. Giữ các chức năng đang có hợp đồng khác như startup cancellation, exact identity, retry cleanup và stale session rejection. Nếu export công khai còn caller live, migrate caller trong mốc hoặc gửi bằng chứng contract cần giữ. Mọi adapter tạm trong mốc cần caller thật và mục gỡ ở M2.
3. Cập nhật Desktop monitor, Stop/Skip, power, close/join và startup failure. Mô hình mới phải chạy qua desktop entry point, acceptance harness và mô phỏng thực tế tương ứng, không tạo lối tắt chỉ để test pass.
4. Cập nhật hợp đồng playback và test hồi quy. Di chuyển regression rebind/Busy cũ thành test hành vi mà thiết kế mới sở hữu, giữ các oracle chống rò phím, lost wake, cursor advancement sai và cross-owner cleanup.

### Nghiệm thu M1

- Start/Pause/Resume/Stop/Skip/Panic/Suspend/finished qua đường chuẩn. Stop khi ngủ dài và trước/sau admission. Owner biến mất, hoặc cửa sổ bị đổi/tái sử dụng, khi có phím giữ và khi không có phím giữ. Mất và hồi phục focus, cùng focus hint stale. Yêu cầu/control/power cũ bị từ chối ở phiên mới. Startup failure chưa có worker và shutdown có worker.
- Journal xử lý Down/Up/mixed sau 1/5/15 phím và cleanup chỉ theo origin. Giữ phân biệt raw zero/partial/QPC lỗi, pre-call cancellation, hậu kiểm quiescence và release uncertainty. Có test fail-closed khi không chứng minh được owner hoặc cleanup. Test cấp phát của đường khỏe vẫn qua.
- Báo map caller/đường đã xóa và operation count trước/sau ở candidate khỏe. Ghi rõ test nào dùng mock và call nào thực Win32. Chưa dùng operation count để tuyên bố latency hoặc gameplay cải thiện.

## 4. M2 — Một đường Prepare và dọn các phần không còn caller

**Trạng thái: Leader đã giao sau khi nghiệm thu M1; M2 implementation đang chờ review.** Caller map, compatibility decisions, kiểm tra và giới hạn chứng cứ được ghi tại [M2 evidence](dispatch-session-redesign-m2.md). Đây là lát cắt toàn pipeline plan → worker → công cụ và phần tương thích, gồm sửa caller và xóa đường cũ cùng đợt.

- Worker nhận đúng một loại plan đã chuẩn bị. Chuyển Desktop, Native API, acceptance scenarios, examples và test helpers về cùng hàm chuẩn bị. Thay `NativeScheduleInput::Raw` tại ranh giới worker. Nếu còn cam kết Rust thực sự cần nhận `RuntimeSchedule`, adapter thuần chuyển sang prepared trước worker và dùng cùng kiểm tra H/G, QPC, budget, cancellation. So lại mã `DispatchExecutor::new_with_cancellation` và Timing Advisory. Advisory có caller sống và oracle riêng. Chỉ gom phần dựng arena chung khi có oracle độc lập cho chuyển đổi tick, workload ngắn/lớn, capacity, lỗi và cancellation.
- Thu hẹp bề mặt Win32 sender. Canonical musical path tiếp tục dùng đúng hợp đồng one-attempt, còn safety Up giữ chức năng của mình. Rà wrapper `*_with_start`, `poll_window_identity_authority() -> bool`, helper raw/retarget và các API nối mock/qualification theo compiler + caller map. Chuyển callsite có hành vi còn cần, xóa wrapper và test chỉ bảo vệ API đã bỏ sau khi đã chuyển hành vi. Tách API thật sự dùng ở test/qualification vào ranh giới phù hợp. Đừng giữ wrapper chỉ để test chính wrapper.
- Bỏ cấu hình waiter một giá trị `WaitGuardMode` khỏi runtime nếu intermediate/diagnostics vẫn biểu diễn đúng `frozen`. CLI/tool đọc artifact có thể giữ parsing cần thiết theo consumer thật. `LegacyTestWideSpin` chỉ giữ nếu có test cần đúng policy này sau khi rà. Khi xóa, thay test coverage bằng fake QPC hoặc override chuẩn ngay trong mốc. Hiện hữu profile/reader của hồ sơ đo cũ vẫn có vai trò nếu còn được hỗ trợ. Giữ bản ghi lịch sử và pins của nó theo provenance.
- Với schema 25, dựng các field compatibility từ trạng thái hiện có ở snapshot/export thay vì ghi duy trì các bản sao trong worker nếu còn bản sao. Giữ missing-QPC provenance và hợp đồng field. Cập nhật parser, docs hoặc fixture cùng thay đổi khi cần duy trì hợp đồng. Mọi thay đổi wire semantics phải có quyết định Leader.
- Giao bảng bằng chứng ngắn **symbol/path | caller trước | caller sau hoặc đã xóa | check xác minh** cho vùng sửa. Bao gồm Rust, Desktop, script PowerShell, feature flags, exports và test support. Rà import, module, enum/variant, feature, counter, branch, cfg, wrapper, test fixture và hướng dẫn cũ do redesign làm mất caller. Default và feature builds đều phải kiểm được. Không dùng `allow(dead_code)`, export `pub` hay để nhánh production cũ để che cảnh báo. Bảo toàn những nhánh còn caller hoặc hợp đồng hợp lệ, nêu caller/contract cụ thể.

**Nghiệm thu:** Desktop Start không build lại plan theo số nốt. Rust admission/qualification bắt đầu từ plan hợp lệ. Kiểm thử H/G tại mốc −1/đúng/+1 tick và tần số QPC không chia hết. Test advisory/cancellation, snapshot/schema, tooling thực tế và hợp âm mixed giữ oracle. Không còn implementation cũ song song, dead branch hay app route hướng tới chúng trong phần thay đổi.

## 5. M3 — Đo đường deadline và qualification

**Trạng thái: chờ Leader giao sau M2.** Giao toàn bộ đo timing, tối ưu phần cận syscall đã định, xác minh Windows và chốt báo cáo.

1. Lấy timing baseline đúng commit/tree ở đầu kế hoạch và candidate exact SHA, cùng toolchain, build profile, features, workload và máy. Dùng provenance hiện có. Nếu runner lịch sử hard-pin nhánh/baseline khác, trình Leader cấu hình mới ngắn gọn dùng lại runner/validator hiện tại. Giữ nguyên các pins cũ và ranh giới ReceiveOnly; tránh tạo bộ test framework song song. M3 chỉ chạy native sau khi Leader xác nhận mapping case/runner và cả hai manifest đúng.
2. Đo authored target D, eligibility E sau sàn H/G, wake/handoff W, QPC pre-call S, completion C. Báo `E−D`, `S−E`, `S−W` khi có W thật, `C−S`, Stop → release, số raw/short, H/G, chord/order, cleanup, CPU/spin, missing/dropped. Báo p50/p95/p99/max. p99.9 có thể báo như mô tả nếu có raw sample và mẫu đủ, đi kèm count; dữ liệu không đủ ghi unavailable. Thiếu QPC hoặc observer loss ghi rõ và loại khỏi phân phối cần chứng cứ, theo validator hiện hành. Tách provenance observer-off và observer-on. `qpc-read-counting` chỉ để đếm hoạt động trong phân tích riêng.
3. Dời công việc publication/format/observer đã đủ dữ liệu ra sau commit hoặc sang consumer khi nó nằm trên đường `Due → SendInput` hay giữ worker ở vòng kế tiếp. Ưu tiên reuse dấu thời điểm và cấu trúc đang có. Giữ record ở các đường replan/failure, cùng semantics loss/accounting và khả năng stop/cleanup. Nếu ra khỏi đường gửi đã đủ qua M1 hoặc không thấy lợi ích, ghi chứng cứ trước/sau và tránh thêm cơ chế.
4. Giữ chính sách waiter, H/G, late wake, thread priority và game sampling profile giống đối chứng. So độ trễ `S−E` theo cặp cùng workload, xem riêng Quiet/Contention và giữ outlier. Dùng tail review prompt hiện tại của runner nếu áp dụng. Bất kỳ sai H/G, wrong-target attempt theo oracle, duplicate Down, chord split là lỗi. Release không xác minh được, mẫu thiếu hoặc provenance gãy phải hiện rõ trong verdict theo runbook. Leader quyết định từ dữ liệu paired và CPU. Nếu dữ liệu đủ cho safety nhưng tốc độ không cải thiện rõ, ghi đúng kết quả; việc giữ cấu trúc đơn giản hơn là quyết định riêng của Leader.

### Đề cương native để chuẩn bị

- **ReceiveOnly Windows**, desktop tương tác đã mở khóa, sink có identity và foreground theo runbook. Candidate Canary `canonical-single` một phiên. Tối đa bảy lượt safety candidate: mất/khôi phục focus, Stop khi đang chờ, pause/resume barrier, process exit, target invalidation và hai kiểm tra retrigger/timing ở 60/120 FPS. Chạy safety trước paired performance.
- **A/B mục tiêu:** một workload `mixed-up-down`, Quiet và Contention. Mỗi cell sáu cặp AB/BA luân phiên, 15 giây mỗi lượt, cùng 60 FPS và margin 500 µs cho hai bản. Tổng 24 lượt đo, tối đa **32 phiên ReceiveOnly** kể cả Canary và safety. Đây là đề cương cho lệnh giao M3, không phải lệnh chạy theo PR mới mở. Tên case và manifest mapping dùng hợp đồng runner. Dừng tại lỗi an toàn đầu tiên. Phân biệt Failed, Inconclusive và chưa chạy. Leader quyết định thêm lượt nếu bằng chứng còn thiếu.
- Raw artifacts, binaries, manifest và command logs để ngoài checkout. Commit chỉ tóm tắt và hash/link có thể kiểm. Sky gameplay thực và OS suspend/resume thực do người vận hành thực hiện trong đợt riêng nếu cần xác nhận sản phẩm. Kết quả ReceiveOnly chỉ xác minh Windows/sink. Muốn kết luận giảm nốt hụt trong Sky phải có bản nhạc/đoạn đã map, cấu hình FPS và đối chiếu số nốt nhận được. Khi không có phép đo đó, ghi rõ game reception chưa kiểm.

## 6. Push, review và merge

Trong mỗi mốc, chạy format/lint và nhóm test phù hợp với phần thay đổi rồi push. Ví dụ từ thư mục `rust/` cho thay đổi Rust/native:

```powershell
cargo fmt --all -- --check
cargo clippy --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings
cargo test --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-features
cargo test --locked -p sky_player --features test-support --test rt_dispatch_no_alloc
cargo xtask check desktop-native
```

Chọn nhóm kiểm tra phù hợp, tránh chạy lặp coverage đã nằm trong cùng command. `desktop-native` áp dụng khi đổi luồng Tauri/DTO/native Desktop. Nếu cần sửa PowerShell/TS, chạy kiểm tra tương ứng repo. Dùng toolchain pin và ghi đúng thư mục chạy. Báo check không chạy, lỗi có sẵn hoặc môi trường thiếu. Không bypass warning/gate hay coi lọc zero test là PASS. Có thể báo lint đỏ thật và push review kèm nguyên nhân đang điều tra. Exact SHA sau mọi commit sửa phải được báo lại.

**Mẫu comment bàn giao trên cùng PR**

```text
M1 READY FOR LEADER REVIEW @<40-char SHA>
Phạm vi đã hoàn tất:
Caller và mã đã xóa:
Hành vi qua Desktop/acceptance:
Kiểm tra: <cwd, exact command, exit, số test hoặc phạm vi>
CI tại SHA này: <pending / failed / passed / chưa có run, link nếu có>
Bằng chứng: <file/URL, baseline/candidate identity, loại mock/native>
Blocker hoặc quyết định còn chờ:
```

Leader review ngay khi có SHA/diff và kết quả local, dù hosted CI đỏ, đang chạy hoặc chưa có run. Kết luận lưu trên PR theo SHA, ví dụ `M1 CHANGES REQUESTED @<SHA>` hoặc `M1 ACCEPTED @<SHA>`. Chấp nhận mốc và kết quả CI là hai thông tin khác nhau. Result đã mất hiệu lực khi sửa phần ảnh hưởng cần chạy lại đúng phần đó. CLI bắt đầu mốc kế tiếp khi Leader giao bằng comment trên cùng PR. Nếu báo thay đổi cần Leader quyết định, ghi nội dung và tiếp tục phần độc lập đã giao.

**Điều kiện đưa ra merge:** M1–M3 đã nghiệm thu trên diff cuối hoặc bằng chứng tương ứng còn hiệu lực, các finding đã xử lý, mã chết đã dọn theo census, contract đang dùng đã cập nhật, và `cargo xtask check all` đã chạy trên final relevant source. Required status **Sky Auto Player — required CI gate** phải thành công trên **exact PR HEAD** tại thời điểm xét merge. Chủ repo quyết định merge. Plan này không giao CLI quyền merge, release hay thay đổi CI.

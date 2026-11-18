# Kế hoạch triển khai lõi playback và cleanup

- Repository `pumni/Sky-Auto-Player`
- Nhánh làm việc duy nhất `refactor/playback-core-lifecycle`
- Baseline `2544daa13eb23fdf03bd922e65b96fcffaba421c`
- Toolchain sản phẩm dùng Rust/Cargo 1.99.1, Windows x64 MSVC. Dùng pin Bun trong repo nếu đụng desktop.
- Trạng thái khởi tạo **Leader giao đợt A**. Các đợt B–D là phạm vi đã định, được Leader kích hoạt lần lượt trong Conversation của draft PR.

File này điều phối thay đổi trên nhánh. Các hợp đồng lâu dài nằm trong các tài liệu sở hữu ở đợt D. Đợt D xóa file kế hoạch khỏi cây cuối. GitHub giữ nguyên bản đã commit để tra cứu từ PR.

## Cách Leader và Implementation Agent phối hợp

- **Leader** chốt semantics, ranh giới quyền, phạm vi review, thay đổi hợp đồng, quyết định từ kết quả benchmark và ngân sách chạy native. Quyết định bổ sung được ghi trong draft PR. Chỉ dẫn của chủ repo ưu tiên cao nhất.
- **Implementation Agent** đọc code tại HEAD, triển khai trọn đợt đang được giao, chạy lint, format, kiểm thử và tạo commit/push. Agent được tự chọn tên biến, tổ chức file, chi tiết kiểm thử hoặc refactor nội bộ nằm trong phạm vi đã chốt. Khi phát hiện mâu thuẫn về chức năng, quyền, cleanup, chấp nhận hiệu năng, IPC, test gate hoặc phạm vi xóa lịch sử, agent ghi bằng chứng và trình Leader quyết định. Có thể tiếp tục các phần độc lập trong cùng đợt.
- Dùng **một draft PR, một nhánh remote** cho mọi đợt. Push tiếp bằng commit mới. Mỗi đợt là một lát cắt hoàn chỉnh qua các tầng liên quan và có thể gồm nhiều commit dễ review. Có thể dùng checkout detached chỉ đọc để đo baseline. Việc hợp nhất main mới, sửa lịch sử, đổi branch, chuẩn bị phát hành hoặc merge PR cần quyết định riêng của Leader/chủ repo.
- Push và gửi handoff ngay khi có artifact review được. **Review bắt đầu theo SHA đã push, kể cả CI đang chạy.** Báo test đỏ và phần chưa chạy. Leader khóa review theo SHA, trả `ACCEPTED A @ <sha>`, `CHANGES REQUESTED A @ <sha>` hoặc `BLOCKED A @ <sha>`, rồi giao đợt tiếp theo nếu đạt. Push thêm phải gửi SHA mới. CI cuối cùng vẫn là bằng chứng merge readiness theo repo.
- Khi môi trường Windows thiếu phụ thuộc hoặc test nền đã lỗi, chạy phần độc lập, đối chiếu trên baseline khi cần, và báo nguyên nhân có bằng chứng. Giữ các gate hiện hành. Nếu cần thay gate vì nó chỉ kiểm tra kiến trúc đã bị thay, mô tả bảo đảm hành vi mới và đưa thay đổi gate vào review.
- Giữ build profiles, signing, updater trust, phạm vi release và giao thức production hiện hành, trừ thay đổi tương thích nêu rõ bên dưới. Dữ liệu test, trace và binary để ngoài cây nguồn hoặc trong artifact của CI. Không chuyển riêng một phần đợt sang PR khác.

Mẫu handoff đăng trong **Conversation** sau khi push.

```text
HANDOFF A @ <full git SHA>
Parent accepted SHA: <full SHA hoặc baseline>
Changes: <các đường sản phẩm đã chuyển và thay đổi hành vi>
Removed: <API/nhánh/tài liệu đã xóa và caller thay thế>
Checks: <command → PASS/FAIL/SKIP, GitHub run link hoặc đường dẫn log>
Windows evidence: <gói evidence và provenance, hoặc NOT RUN>
Review points: <races, invariant đã kiểm, các quyết định hoặc blocker>
```

## Quyết định kỹ thuật đã chốt

| Chủ đề | Hợp đồng dùng khi triển khai |
| --- | --- |
| Đường sản phẩm | Một phiên vật lý, một worker gửi phím, một HWND/PID/process object bất biến từ Arm đến cleanup. Recovery nhận quyền sau khi worker đã join. Một đường musical packet và một đường safety Up, dùng chung primitive Win32 thực hiện tối đa một syscall mỗi lần gọi. |
| Nhạc và thời gian | Giữ executor 15 slot, prepared schedule, QPC deadline tuyệt đối, hợp âm thực thi tối đa một gói và thứ tự Up trước Down trong gói. H tính từ completion Down và G từ completion Up hợp lệ. Duy trì thứ tự tác giả, chọn Up độc lập và semantics hiện hành khi thức muộn. Modifier chỉ chặn Down, không pause timeline. Hủy/power/lease/identity lỗi tiếp tục ngăn gói mới. |
| Admission | Chặn sớm theo control. Kiểm tra modifier cho Down, process continuity đã bind và foreground HWND/PID mới, rồi đọc control lần cuối sát sender. Giữ kiểm tra deadline và bằng chứng QPC hợp lệ trước/ở sender. Fresh foreground không nhận từ callback/cache. Tách focus mất tạm thời có thể recover và origin bị mất phải terminal. |
| Liveness | `OpenProcess` chỉ lấy `PROCESS_QUERY_LIMITED_INFORMATION \| SYNCHRONIZE` cho authority playback. Dùng zero-time wait trên handle đã bind để phân biệt sống, đã thoát và lỗi truy vấn. Dùng creation time/image tại bind cho định danh. Thiếu quyền/handle khiến Arm lỗi rõ. Process exit trong long idle/Pause cần làm worker dậy hoặc được phát hiện theo cadence hữu hạn. |
| Stop và các race | Duy trì mapping Stop, Skip, Panic, Suspend và supervisor expiry qua DTO hiện có. Dùng thứ tự và nguyên nhân hiện hành, lấy test hành vi làm căn cứ. Lỗi transport/cleanup không bị cơ chế xuất UI che khuất. Nếu kết quả khi nhiều lệnh đến đồng thời đang mâu thuẫn giữa các entry, đưa case cụ thể cho Leader. |
| Transport | Ghi journal trước syscall. Complete có QPC hợp lệ mới commit musical action. Gói đã thử syscall không được phát lại. Dừng bài ở anomaly đầu tiên, gồm zero, short, count mâu thuẫn hoặc QPC lỗi. Nếu Down-only raw zero được chứng minh nhất quán, bỏ đúng nghĩa vụ mới chưa chèn, giữ nghĩa vụ cũ để cleanup. Không suy ra prefix đã chèn từ short count ở bản đầu. |
| Tách nghĩa vụ cleanup | Trong Drain, giữ hai mask rời nhau. `needs_up` là phím còn thiếu transport đủ điều kiện. `needs_probe` là phím đã chèn đủ ở foreground nhưng chưa xác minh theo probe. Probe trái ngược/inconclusive giữ mask. Khi probe tin cậy, chỉ giải quyết phần đã có cả transport hợp lệ và quan sát phù hợp. Gói rỗng trả ngay mà không gọi QPC/foreground/transport cho cleanup. |
| Cleanup hữu hạn | Giữ backoff 15/50/100 ms và tối đa bốn raw Up call trong một lượt drain của worker. Đếm riêng bị chặn, đã admit, raw call được ghi nhận và unwind khi raw chưa rõ. Retry vật lý chỉ cho `needs_up`; nếu chỉ còn `needs_probe` thì tiếp tục probe hữu hạn khi điều kiện hợp lệ. Tái xác minh origin sau mỗi delay, ngay trước lần gửi. Nếu Stop đến khi đang cleanup vì Pause, nâng mục đích của cùng lượt drain; ngân sách và bằng chứng được giữ. |
| Stop khi game ở nền | Cho tối đa một safety Up best-effort khi origin cũ còn sống và đã xác minh. Phím lấy từ journal phiên. Lượt ở nền dùng một phần ngân sách drain và không tính vào bằng chứng nhả của target. Gói nhạc luôn cần foreground. Quan sát keyboard trên Windows và raw count đứng riêng với việc game nhận note. |
| QPC hỏng và recovery | Safety Up đã được xác minh origin vẫn có thể gửi có giới hạn khi QPC lỗi. Phiên ghi lỗi thời gian và kết thúc; barrier resume chỉ dựa trên QPC thật sau xác minh. Release chưa xác minh giữ residual có session ID, profile và original authority. Sau join, một lần quay lại foreground của đúng origin có thể kích hoạt một lượt recovery với tối đa một raw Up call và probe hữu hạn. Recovery gửi Up cho `needs_up`, probe `needs_probe`, không đồng thời với worker khác. Origin mất hoặc recovery vẫn không xác minh được thì giữ quarantine với lý do rõ, giữ khả năng restart hiện có. |
| UI, quyền phát và telemetry | Worker trả terminal result có kiểu rõ. Shell join rồi trả hoặc chuyển quyền phát sang recovery, commit terminal theo session ID rồi mới phát presentation event. Preserve get-playback-status, session ID fencing, Auto Play/Skip/Replay và recovery khi UI mất event. Payload IPC hiện có giữ tương thích. Forensics chi tiết đi theo chế độ diagnostic/qualification; giá trị chưa thu biểu diễn unavailable. |
| Chính sách Windows | Playback dùng public `SendInput` cho chơi nhạc. Cho phép quyền liveness tối thiểu ở A và `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, …, WINEVENT_OUTOFCONTEXT)` trong adapter chuyên trách ở C. Callback chỉ publish hint/wake, có message loop, tháo hook trên luồng đăng ký và kiểm soát event đến muộn. Khi watcher lỗi dùng poll dự phòng. Game files, memory, injection, in-context/keyboard hooks, debugger và anti-cheat nằm ngoài phạm vi; không mở Raw Input sink toàn cục hay API ntdll. |
| Thay trạng thái và tương thích | Dùng shape hẹp trong production và test hành vi ở cùng entry. Thay assertions chỉ dựa tên/đường cũ khi migration thay cấu trúc. Một API/thành phần cũ chỉ được xem là xóa xong khi caller/cấu hình/test và docs tương ứng đã chuyển. Native calibration và acceptance executable là các caller thật. |

## Đợt A — Quyền sở hữu phiên, điều phối và final gate

**Được giao ngay.** Đi qua desktop composition, session/target, worker, Win32 authority, mock và acceptance. Kết quả là production nhận một target đã bind trước Arm và thực hiện gate ngắn hơn tại entry point thật.

1. Đưa dữ liệu bind target vào constructor hoặc một giá trị đã xác minh được move vào worker. Chuyển desktop, các test liên quan và native acceptance. Mô phỏng target invalidation bằng seam đúng phạm vi test hoặc production shutdown, thay vì giữ API rebind đại trà.
2. Dùng handle liveness với quyền đã chốt. Bỏ phụ thuộc vào `exit FILETIME` lúc process còn sống. Truyền quyền/target từ Arm đến sản phẩm thật, bảo toàn cleanup origin, cả khi window thay đổi hoặc PID/HWND bị tái sử dụng. Cập nhật đúng phần quyền handle trong `SECURITY.md` cùng thay đổi audit/unsafe nếu cần.
3. Rút final admission, đưa control/power/focus vào thứ tự xác định, bỏ vòng kiểm tra mô tả cùng một owner, giữ phép đọc Win32 ở cuối. Đo số đọc atomic, foreground, PID và liveness trên gói Down, Up và mixed bằng test seam. Giữ đường waiter hiện tại và QPC samples required. Giữ nguyên thuật toán spin trong A để so sánh theo một thay đổi kiến trúc. Bản tối ưu QPC đã phân tích trước đó là một thử nghiệm riêng do Leader quyết định sau khi có ứng viên.
4. Bao phủ focus mất sát deadline, target chết trong long gap, Stop/Pause/Skip/Panic/suspend quanh Arm và wait, lease hết hạn, modifier gặp mixed Up/Down, full chord và custom profile. Chạy kiểm thử qua entry production và mock phù hợp. Chứng minh no-alloc của đường stage/send khỏe.
5. Với release ledger và types được cleanup cũ dùng, chỉ xóa ngay phần đã hết caller. B chuyển phần còn lại. Ghi chính xác các cầu nối tạm và nơi B sẽ xóa trong handoff.

## Đợt B — Sender, journal và một cleanup reducer

Chuyển trọn đường từ packet/Win32 outbound tới executor commit, safety Up, startup/terminal/resume và report đưa ra cho desktop.

1. Cấp phát gói trước wait, chỉ một primitive syscall. Mỗi packet mang raw evidence độc lập với hợp lệ thời gian. Ở biên unwind giữ nghĩa vụ in-flight đủ để cleanup chạy. Journal dùng owner cấp phiên, loại bỏ per-key owner copy và thu hẹp đường refill. Nếu mô hình dưới Win32 có nhiều consumer, dùng protocol/primitive chung, giữ semantics riêng của calibration.
2. Dùng cùng reducer cho startup journal rỗng, manual Pause, focus return, Stop, suspend, packet anomaly và panic. Áp dụng bảng đã chốt cho `needs_up`/`needs_probe`, authority, QPC và retry. H/G và chính sách Resume nằm ở edge scheduler, ngoài retry transport.
3. Chuyển nguyên nhân terminal sang kiểu rõ để C dùng. Xóa nhánh tiếp tục phát sau anomaly, các đường unguarded sau khi startup đã chuyển, các wrapper/raw retry không có caller thật và ledger owner theo slot đã hết vai trò.
4. Test matrix bắt buộc gồm raw zero, short, full, Win32 error, mất QPC trước/sau, panic trước/sau syscall và trước commit, ngắt Stop ở giữa Pause Drain, retry bị chặn do origin đổi, target nền trong toàn bộ Drain, probe Held/Inconclusive, layout/profile và zero-mask. Test bảo đảm không replay onset, không giải phóng phím ngoài journal, giao đúng retry budget và giữ bằng chứng raw đã có.
5. Ứng viên xóa đã thấy ở baseline là `emergency_release_canonical`, đường `FullInstrument` không có consumer production trong repo, `release_terminal_tracked_scope` chỉ test gọi và các wrapper raw/legacy `key_down/key_up`. Xác minh cả bin/example/scripts/feature/re-export trước khi xóa. Nếu có consumer ngoài repo được hỗ trợ, trình Leader khi xác định được. `send_prepared_physical_packet_once_at_target` có caller native calibration; giữ behavior, provenance và timing của nó.

## Đợt C — Retirement, recovery, focus watcher và bằng chứng Windows

Chuyển từ kết quả terminal trong worker qua join/shell/IPC, lifecycle recovery, event/policy và công cụ qualification. Đợt này kết thúc với sản phẩm chạy end-to-end.

1. Shell dùng typed terminal result. Join trước khi chuyển quyền vật lý. Commit terminal/retirement theo session ID trong state owner của shell, rồi mới dispatch UI. Tháo vòng retry clear active-slot chạy trên presentation. Giữ idempotence và native query fallback. Nếu event cũ có thể đến sau khi bài mới đóng, giữ kết quả cũ đủ cho reconciliation theo ID ngay trong state hiện có. Không đổi giao thức công khai để che race.
2. Dùng một quyền độc quyền cho recovery, giữ origin và hai mask còn nợ. Cài focus watcher theo policy đã chốt và dùng watcher để đánh thức từ long idle/mất focus và recovery. Event là tín hiệu để kiểm tra lại, không làm chứng cứ sender hoặc mở khóa một target khác. Kiểm thử event cũ, message-loop failure, poll fallback, teardown và app đang foreground. Khi active, việc render/telemetry chậm không được giữ lease/control.
3. Sửa `SECURITY.md`, summary trong `AGENTS.md`, tài liệu playback/architecture cần thiết và security audit cùng code. Scope exception của `SetWinEventHook` tới đúng adapter nguồn hoặc symbol được review. Wrapper khóa event và flags. Test positive approved site, negative ngoài site, in-context/keyboard hook, dangerous access đang cấm và teardown. Dùng architecture allowlist cho unsafe boundary. Giữ security historical baseline theo mục đích gốc, không dùng để miễn call site tính năng.
4. Tách forensics production khỏi đường sau send và khỏi quyền đóng phiên. Khi diagnostics/qualification cần dữ liệu, giữ đúng schema hoặc báo unavailable đúng contract. Xử lý sự kiện nhiều và observer/desktop treo trong tests. Nếu đề xuất bỏ watchdog, timer calibration, đổi placement hoặc power policy, đưa evidence và patch tách biệt cho Leader. Liveness/lease timeout trên Arm/Pause/long gap và suspend power callback vẫn là ca bắt buộc.
5. Chuẩn bị qualification Windows từ exact baseline/candidate. Dùng entry hiện hành nếu nó hỗ trợ đúng provenance. Nếu bị pin vào milestone cũ, thay/điều chỉnh profile vào cùng harness với tên `PlaybackCoreLifecycle`, dùng manifest có baseline SHA, candidate SHA, tree, Rust, target, profile, features, lockfile và hash binary. Không đổi pins lịch sử để hợp thức hóa dữ liệu cũ. Nộp build/manifest/runner dry checks trước. Leader ghi phạm vi native ReceiveOnly cụ thể trên PR dựa vào candidate đã review; gameplay thật do chủ repo điều khiển.

## Đợt D — Kiểm tra cuối và dọn mạnh tay tài liệu

Đây là **đợt cuối**. Hoàn thiện xóa code trước rồi làm một commit documentation cleanup riêng trong cùng nhánh.

1. Đối soát `git diff <baseline>...HEAD` với call graph từng entry sản phẩm. Truy các `pub use`, test-only variants, feature no caller, DTO fields không có consumer, dead suppressions, scripts/cases và wrappers chuyển tiếp. Xóa chưa dùng khi có bằng chứng và không phá contract có caller. Dùng Clippy và các compile matrix bên dưới để hỗ trợ; kiểm tra `pub` exports thủ công vì compiler không báo unused public API.
2. Rà `docs/playback-engine.md`, `docs/playback-verification.md`, `docs/architecture.md`, `docs/INDEX.md`, `docs/HISTORY.md`, root/docs `AGENTS.md`, README và các guide có mô tả playback. Tìm các tên lifecycle/owner/retry trước redesign, stale CLI commands, schema, thresholds, milestones, selectors và docs chỉ tồn tại để giải thích workaround cũ.
3. **Xóa khỏi cây hiện hành** các plan đã thi hành, notes trùng, nội dung còn mô tả code đã xóa, câu lệnh chỉ chạy ở branch/head cũ, trích log và phần replay quyết định lịch sử. Nếu bằng chứng có giá trị provenance, dùng một immutable commit link trong `docs/HISTORY.md`; tránh tạo bản sao archive trong cây hiện hành. Giữ các hợp đồng độc lập về release, trust, packaging, licensing, sheet compatibility và UI có caller thật.
4. Mỗi chủ đề còn đúng một nơi sở hữu. `playback-engine.md` là lifecycle/invariant hiện hành. `playback-verification.md` là cách chạy/đọc bằng chứng có hiệu lực. `SECURITY.md` là policy. `AGENTS.md`/INDEX chỉ dẫn ngắn, không chép lại hợp đồng. Đảm bảo local links còn trỏ đúng. Chạy audit docs và quét lại references. Nộp bảng ngắn `deleted → reason → immutable source nếu cần` trong PR, không ghi thêm report dài trong repo.
5. Xóa file kế hoạch này khỏi cây cuối. Link từ PR dùng SHA commit ban đầu hoặc revision Leader mới nhất. Nộp exact final HEAD, trạng thái CI, kết quả native và danh sách uncertainty. Leader sign off merge readiness; chủ repo quyết định merge.

## Lệnh kiểm tra và quy tắc nghiệm thu

Chạy từ repo root trên Windows sau khi checkout nhánh, tùy phạm vi từng đợt. Handoff ghi lệnh thật và exit status. Trong dev loop ưu tiên focused tests. Ở final chạy verifier chuẩn và kiểm tra CI exact PR HEAD. Tauri tests có asset/tool setup riêng, theo lệnh repo ở checkout hiện tại.

```powershell
git fetch origin
git switch --track origin/refactor/playback-core-lifecycle
rustc --version
cargo --version
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings
cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --features sky_player/test-support -- --test-threads=1
cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc
cargo test --locked --manifest-path rust/xtask/Cargo.toml audits::security::tests
cargo check --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance
cargo xtask check static --skip-supply-chain
git diff --check
```

Khi đụng shell hoặc UI, chạy test tích hợp native liên quan với `tauri-test` và setup fixture desktop của repo, consumer settlement/retirement, Auto Play, command/status và kiểm tra bindings nếu thay DTO. Chạy các lệnh Bun của ứng dụng desktop nếu có thay đổi TS. Khi final, chạy `cargo xtask check all` với công cụ supply-chain đã pin. Một lần static skip supply chain chỉ xác nhận phần static đã chạy.

**Gate chức năng.** Một writer vật lý; zero Down khi đã quan sát stop/suspend/authority invalid; zero Down ngoài foreground; mỗi onset tối đa một lần attempted send; healthy chords không split; Up trước Down; zero H/G violation; zero lần nhả ngoài nghĩa vụ; kết quả terminal cùng release/quarantine đúng cả khi ping UI thất bại. Tình huống thiết kế yêu cầu giữ residual phải hiển thị là chưa xác minh và chặn successor. Không báo kết quả đạt nếu provenance, coverage hoặc phép đo bắt buộc thiếu.

**Bằng chứng hiệu năng.** So exact SHA trên cùng Windows/máy/target, Rust 1.99.1, profile `dist`, cùng features/settings. Dùng ReceiveOnly foreground độc lập. Báo p50/p95/p99/max cho eligible→pre-call, wait→pre-call, pre-call→completion, Stop→Up→release-verified, worker CPU/spin, số Up lặp và worker joined→Start kế tiếp. Cùng workload mixed, dense/maximum chord, repeated key, và long idle trong Quiet/Contention, với các cặp AB/BA theo stage Leader giao. Báo dữ liệu thiếu, raw counts, sai khác theo pair và biến động. Giữ nguyên ý nghĩa các tail-review prompt của runner; Leader kết luận từ kết quả paired. Không gán outcome sender thành độ chính xác gameplay.

## Điểm vào mã nguồn để mở đúng nơi

- Shell ownership và retirement: `apps/desktop/src-tauri/src/native_runtime/playback/{session,retirement,presentation}.rs`, `apps/desktop/src-tauri/src/app_state.rs`.
- Native authority và worker: `rust/crates/sky_player/src/engine/{session,target,shared}.rs`, `engine/worker/{admission,dispatch_loop,dispatch_loop_scheduler,control,cleanup}.rs` và `engine/worker/cleanup/finalize.rs`.
- Timing và Win32: `rust/crates/sky_dispatch_core/src/executor.rs`, `rust/crates/sky_dispatch_win32/src/focus/window_identity.rs`, `src/input/{packet,raw,physical}.rs`, `src/input/tracked/{state,packet_send,cleanup}.rs`.
- Kiểm chứng/policy: `rust/xtask/src/audits/security.rs`, `.config/rust_architecture_allowlist.json`, `scripts/qualify_dispatch_core.ps1`, `scripts/build_dispatch_core_harness.ps1`, `rust/crates/sky_player/tests/rt_dispatch_no_alloc.rs`.

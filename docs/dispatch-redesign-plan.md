# Kế hoạch triển khai vòng đời dispatch

**Nhánh duy nhất** `refactor/playback-prespin-admission` · **Base ban đầu** `55847bbdb4ab1564e7b769d76023b8833d0fa568` · **Gate hiện tại** G0, chuẩn bị và triển khai Đợt 0–1 trên một draft PR vào `main`.

Tài liệu này là bàn giao công việc trên nhánh. Các hợp đồng có hiệu lực nằm ở [playback engine](playback-engine.md), [Windows verification](playback-verification.md) và [Security Policy](../SECURITY.md). Kế hoạch thu gọn còn một tài liệu tạm và sẽ được xóa trong lượt cleanup cuối. GitHub giữ lịch sử giao việc và quyết định theo SHA.

## 1. Quyền quyết định và cách phối hợp

| Vai trò | Trách nhiệm và quyền |
| --- | --- |
| Chủ dự án | Chấp thuận máy Windows, ReceiveOnly sink và phạm vi gửi vật lý. Quyết định merge, phát hành và mọi thay đổi có ảnh hưởng ngoài phạm vi thử nghiệm đã duyệt. |
| Leader | Giao gói việc, review theo SHA, chốt biến thể A/B/C, ngân sách tuổi bằng chứng và QoS. Quyết định thay đổi hợp đồng, profile/baseline, rollout, đợt tiếp theo và danh sách tài liệu cần loại bỏ. Xác nhận các quyết định lớn trong draft PR. |
| Implementation agent trên CLI | Triển khai gói đã giao, chọn chi tiết cơ học trong phạm vi, chạy lint/format/test, chuẩn bị và chạy thí nghiệm được duyệt, push vào nhánh duy nhất. Báo blocker kèm bằng chứng và phần việc độc lập còn làm được. Chuyển các quyết định quan trọng cho leader. |

**Quy trình GitHub**

1. Dùng một nhánh và một draft PR trong tất cả các đợt. Mỗi gói có thể gồm vài commit liên quan, bao trùm lab/harness, native adapter, worker, test và bằng chứng nếu cần. Gom theo kết quả có thể review. Tránh chia gói theo số file. Không force-push hoặc tạo PR/nhánh phụ. Detached worktree để build baseline được phép.
2. Sau mỗi push có diff hoặc evidence đáng review, CLI báo ngay trên PR hoặc chuyển cho chủ dự án để đăng cùng SHA. Leader review diff và phản hồi theo đúng SHA khi CI còn chạy, pending hoặc đỏ. Đính kèm trạng thái CI và mọi lỗi biết được. Review sớm và xác nhận merge readiness là hai mốc riêng.
3. Leader ghi một trong ba kết quả trên PR cho đúng SHA, gồm cần sửa, chấp nhận checkpoint, hoặc mở gate tiếp. Nếu developer phát hiện cách làm hiện tại cần đổi quyền gửi, thay workload, đổi baseline, thay chính sách QoS mặc định hoặc vượt scope, dừng phần phụ thuộc quyết định đó và tiếp tục phần độc lập.
4. Giữ tài liệu này là hàng đợi duy nhất trên nhánh. Quyết định và báo cáo theo SHA đi trong PR. Đưa log, trace, manifest và dữ liệu mẫu dài vào artifact hoặc vị trí evidence do chủ dự án duyệt, ngoài cây source. PR chứa bảng tóm tắt và liên kết. Không đưa thông tin nhận dạng máy riêng, token hoặc path cá nhân vào PR công khai.

**Từ khóa điều phối**

- `G0 START` cho phép làm Đợt 0–1 theo mục 3. Tạo nhánh và draft PR không tự cấp quyền gửi input thật.
- `G1 ACCEPT <sha>` là quyết định của leader sau khi đã xem số liệu Đợt 0–1. Kèm lựa chọn A/B/C theo loại gói, QoS, ngân sách precheck và các phần được phép mở tiếp. Thiếu dòng này thì chưa mở Đợt 2.
- `NEXT <đợt> <sha>` mở gói tiếp theo khi invariant, diff và evidence phù hợp. Có thể mở phần không phụ thuộc CI khi đã đủ căn cứ review. Cổng kiểm tra cuối vẫn theo [AGENTS.md](../AGENTS.md).

## 2. Bất biến và giới hạn triển khai

- Một worker sở hữu physical send, executor 15 khe, candidate và journal của phiên. Chỉ transfer recovery sau khi worker join. Session ID cũ không được giải phóng reservation của phiên mới.
- Giữ profile 15 phím, generation, chord Down nguyên và mọi Up trước Down trong gói hỗn hợp. H/G neo vào QPC hoàn tất hợp lệ. Down bị modifier chặn không làm dịch epoch, không ghim deadline Up. Up đủ điều kiện vẫn được xét độc lập.
- Một lần thử raw send cho mỗi musical onset. Raw-zero, partial, dữ liệu mâu thuẫn và QPC hỏng đi theo hợp đồng cleanup hiện hành. Không retry Down. Giữ `needs_up` và `needs_probe`, đúng origin, tối đa 4 raw Up mỗi worker drain và 1 mỗi recovery attempt. `Drop` chỉ thu hồi tài nguyên hoặc công bố terminal của supervisor, không gửi phím.
- `SendInput` là đường gửi duy nhất. Không thay game, file game hoặc anti-cheat. Không thay namespace quyền, updater, signing, release hay asset. `SECURITY.md` và hợp đồng foreground HWND/PID của production giữ nguyên trong Đợt 0–1. Có thể bỏ `GetProcessTimes` lặp từ authority đã bind, vẫn giữ liveness và truy vấn HWND/PID tại admission production.
- B/C là cấu hình thí nghiệm chỉ có đường vào ReceiveOnly được hậu kiểm identity. Giữ đường A làm control. Không lộ một cờ config để phiên game bật B/C. Nếu thêm API QoS, giới hạn vào process của tool và chạy audit. `SetProcessInformation` nằm trong module Threading hiện đã được cho phép, nên chỉ sửa phạm vi audit nếu có bằng chứng test yêu cầu và leader chấp thuận.
- Rust/Cargo 1.99.1, Windows MSVC x64, `windows-sys`. Giữ ABI `BOOL` dạng `i32` ở biên, dùng raw borrow đúng lifetime, tránh unwind qua callback. QPC tick checked và không thêm allocation, lock, quan sát UI hoặc log ở đường gửi lành. Kiểm chứng layout trước khi dùng padding hoặc alignment nội bộ.

## 3. G0 — gói được giao đầu tiên

**Kết quả gói.** Push một chuỗi commit review được trên nhánh này, gồm lab có thể lặp, control A đã rút gọn, ứng viên B chỉ dành cho ReceiveOnly, test theo ranh giới và báo cáo evidence đã có. C chỉ triển khai khi bằng chứng cho thấy chuyển scan modifier ra sớm là điểm yếu, hoặc khi leader yêu cầu. CLI có thể chuẩn bị baseline và code trên cùng nhánh trong lúc chờ quyền chạy vật lý. Việc đo quyết định lựa chọn, không trì hoãn push implementation và review sớm.

### Đợt 0 — đo môi trường Windows và dựng đối chứng

| Việc | Kết quả cần cung cấp |
| --- | --- |
| Chốt nhận dạng môi trường | Bản Windows 11/build, nhóm CPU, QPC frequency, nguồn AC/battery và power plan, fixed spin, backend timer thực tế, MMCSS/thread QoS đã acquire, compiler/profile/features và SHA/binary hash. Ghi các điều kiện đủ để lặp với cấu hình riêng tư phù hợp. |
| Tách visibility của app | Mỗi case giữ fixture Receiver ở foreground. Cửa sổ của tool lần lượt visible nhưng không foreground, fully occluded, minimized. Giữ cùng tải, đường timer, thread QoS, spin, âm thanh và nguồn điện trong cặp đo. Ghi rõ cách tái tạo từng condition. |
| A/B process QoS | A là mặc định. B chỉ đặt bit `IGNORE_TIMER_RESOLUTION` trong `ControlMask` với bit tương ứng của `StateMask` bằng 0. Backend production không xin `timeBeginPeriod` qua waiter. Đừng thêm yêu cầu timer mới để tạo hiệu ứng. Nếu thử B thành công về API nhưng hiệu quả không lặp lại ở occluded/minimized, đo C-QoS riêng với chỉ `EXECUTION_SPEED` tắt ở cấp process. Không bật hai cờ cùng lúc trong phép đo chẩn đoán. |
| Cô lập và đọc kết quả | Dùng process test riêng hoặc vòng đời khôi phục cấu hình có thể kiểm tra. Acquisition failure là nhóm riêng. Báo p50/p95/p99/max của kernel wake, deadline wake, eligible-to-pre-call, CPU/spin và số mẫu. Tách no-op, inconclusive và hiệu ứng lặp lại. Ghi độ nhiễu cặp baseline để leader chốt biên hiệu năng trước khi chọn một biến thể thắng. |

### Đợt 1 — admission nhanh và bằng chứng

| Biến thể | Hợp đồng thực thi |
| --- | --- |
| A · đối chứng | Không đọc lại creation time trên mỗi gói. Giữ scan modifier cho Down, zero-time process wait, foreground HWND/PID và control cuối. Dùng chung đường journal/sender với các biến thể. Truy vấn admission mục tiêu sau crossing 8 cho Down/mixed, 3 cho Up-only. |
| B · precheck | Timer thức theo fixed spin window. Chờ tới `eligible_qpc - precheck_lead` mới bắt đầu PID, modifier khi có Down, lease và liveness. Điểm thử khởi đầu `precheck_lead = 200 µs`, tuổi bằng chứng tối đa thử 500 µs, đóng băng trong một cặp. 150/250 µs chỉ là sweep chẩn đoán báo riêng. Các giá trị shipping cần leader chốt. |
| B · liveness và suffix | Dùng wait-any với `[process, control, timer]`. Chỉ mượn bằng chứng của đúng handle và đúng lần timer thắng. Nếu đã due, không qua multi-wait hoặc quá tuổi, lấy zero-time proof rồi dùng đường chặt hơn khi stale. Capture revision trước nhóm scan, tính expiry từ mẫu bảo thủ trước quan sát đầu tiên và lease. Tại crossing, đọc foreground HWND, đo pre-call QPC thật, kiểm tra expiry, đọc một Acquire thu hồi nếu semantics publisher đã được chứng minh, ghi in-flight rồi gửi. Báo count 1 truy vấn admission. Tách chi phí QPC và `SetLastError`, giữ bằng chứng lỗi sender. |
| B · dự phòng | Precheck bị lỡ hoặc stale hay provenance thiếu thì đi A trước raw send khi control và QPC vẫn hợp lệ. Thu hồi, race chưa giải được về quyền gửi hoặc QPC bắt buộc lỗi thì chặn theo outcome hiện có. Không coi HWND trùng hay Raw Input/WinEvent là đủ bằng chứng identity. Không làm mới proof bằng cách lấy timestamp sau khi chờ. Giá trị QPC tại pre-call không bảo đảm OS sẽ không preempt ngay sau đó. |
| C · lai | Chỉ khi cần sau probe của B. Đưa PID/liveness ra trước spin khi đủ bằng chứng, giữ scan modifier đồng bộ sát gửi nếu có Down. Mục tiêu 6/1 truy vấn admission sau crossing. Nếu PID/liveness mới là phần chưa đạt, giữ kiểm tra tương ứng của A. |

**Đơn giản hóa có thể review.** Trong `admission.rs`, gom wrapper Down/Up về một quyết định typed theo loại gói và cắt các bước ánh xạ reject hoặc kiểm tra dữ liệu bất biến lặp. Trong `dispatch_loop.rs`, gom control context trùng, dùng lại mask đã stage và bỏ phép kiểm tra presence đã được bind và adapter xác minh. Giữ mapping Stop/Skip/Suspend, phép xác thực origin và các lần đọc có vai trò race khác nhau. CLI nộp bảng trước/sau về entry point, branch đường healthy và nhánh trùng đã xóa trong cả hai file. Việc chuyển mã sang module khác không tính là cắt bỏ. Bản vá thử cũ nằm ngoài nhánh mới và không phải đầu vào mặc định.

**Race và test bắt buộc.** Kích hoạt modifier giữa 5 lần đọc hoặc ngay sau scan, đổi focus sau precheck và sau foreground query, mô phỏng HWND cùng giá trị với owner khác, process exit khi timer vừa trả hoặc trong spin, stale proof do preemption, control/lease/suspend cạnh tranh với crossing, partial/raw-zero và lỗi QPC sau send. Kiểm tra cả Down-only, Up-only và mixed. So receiver log, journal và raw attempts. Nếu B tạo Down dưới modifier mới hoặc rủi ro owner không được chứng minh đủ, đánh dấu B không đạt cho nhánh liên quan và đưa phương án C/A cho leader.

## 4. Bằng chứng, quyền chạy máy và cổng G1

**Quyền ReceiveOnly.** Trước một phép chạy vật lý, ghi trong draft PR hoặc kênh chủ dự án xác nhận được về máy Windows, identity của fixture thuộc dự án, phạm vi scan-code/input, và cửa sổ phụ nếu bài race cần đổi foreground. Dùng các fixture được nêu trong phạm vi đã duyệt. Chỉ bật `-OwnerApprovedReceiveOnlyRun` trong đúng ngữ cảnh được phê duyệt. Nếu chưa có quyền hoặc chưa có máy, push kết quả compile, dry-run và test không phát input rồi báo `NOT RUN` cho native.

**Hai tuyến evidence.** Đợt 0–1 là A/B chẩn đoán cho cùng harness và cấu hình trên nhánh mới, có SHA/tree và binary hash. Profile `PlaybackCoreLifecycle` hiện khóa lineage ở `refactor/playback-core-lifecycle` và baseline `2544daa13eb23fdf03bd922e65b96fcffaba421c`. Đừng đổi branch binding, baseline, evidence schema hoặc mượn nhánh cũ để làm cho phép đo nhánh mới hiện chữ PASS. Nếu cần qualification chính thức, leader quyết định adapter hoặc profile kế nhiệm qua PR. Giữ chỉ số `worker_joined_to_next_start_us` là INCOMPLETE khi baseline/receiver evidence chưa đủ. [Profile](../scripts/dispatch_qualification_profiles.psm1) · [Runbook](playback-verification.md)

**Ma trận gửi leader**

| Nhóm | Báo cáo |
| --- | --- |
| Process QoS | Mỗi hàng theo visibility, load, policy, số mẫu/cặp, hiệu lực API, p50/p95/p99/max wake và CPU/spin. File hoặc artifact có raw rows. |
| Admission A/B/C | Mỗi hàng theo workload, load, loại gói, biến thể, mẫu, `eligible→pre_call`, `wake→pre_call`, `pre_call→completion`, p50/p95/p99/max, API count trước/sau crossing, tuổi precheck và fallback. Tính percentile B trên tất cả gói gán vào B. |
| Timing workload | Dùng 5 workload có sẵn và Quiet/Contention. Mốc so có thể lặp là 6 cặp AB/BA, mỗi mẫu 15 s. Thử nhanh và race study ghi riêng. Nếu không đủ mẫu, ghi đúng lý do và tập đã chạy. |
| Correctness | H/G, Up khi modifier Held, send attempt một lần, sai đích, race control, số raw Up, residual, quarantine và các điểm chưa đo. Lỗi quyền gửi hoặc sai H/G là blocker. |
| Chất lượng code | Lệnh, exit code, số test, `rt_dispatch_no_alloc`, CI link/status tại cùng SHA, thống kê admission/dispatch branch và diff chú giải. |

**Lệnh kiểm tra ở root**. CLI dùng môi trường Windows MSVC đã pin và chạy theo từng checkpoint phù hợp.

```powershell
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo check --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance
cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance -- -D warnings
cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc -- --test-threads=1
cargo xtask check static
pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
git diff --check origin/main...HEAD
```

Đính kèm kết quả focused tests cho seam đã đổi và nguyên nhân nếu lệnh nào cần bỏ qua. Không ghi PASS khi lệnh chưa chạy. Với thay đổi shell lifecycle theo đợt sau, thêm all-features shell tests theo runbook. `cargo xtask check all` và required CI trên exact PR HEAD được dùng khi xác nhận merge readiness. PR có thể được review trước cả hai.

**Gate G1.** Leader xem native A/B, độ nhiễu baseline, tình trạng QoS, kết quả race, `rt_dispatch_no_alloc` và đơn giản hóa. Sau đó ghi SHA nào được chấp nhận, A/B/C được chọn theo Down/Up/mixed, trạng thái QoS và điều kiện đo tiếp. Các ngưỡng Gemini p95 50 µs và p99 150 µs là mục tiêu thử nghiệm. Không chấp nhận kết luận từ nhánh nhanh riêng hoặc khi thiếu evidence rủi ro. Nếu chưa đủ, yêu cầu đúng phép đo thiếu trên cùng PR. Dừng tại G1 trước Đợt 2.

## 5. Các gói kế tiếp sau quyết định của leader

| Gói | Kết quả gộp và điều kiện nghiệm thu |
| --- | --- |
| Đợt 2 · waiter, staging và spin | Đo calibration sau scheduling guards rồi thử phép đổi tick/100 ns và adaptive spin độc lập trên QoS đã chọn. Không để spin window gia hạn proof. Chỉ giữ estimator nếu p99 tốt hơn với mức CPU/spin đã duyệt. Cắt đường wait/dispatch trùng, giữ 15 khe, H/G và race evidence. |
| Đợt 3 · Pause và modifier | Pause pre-roll giữ countdown, Resume dùng pause interval hiện hành. Chờ mềm `Held` có deadline, `Inconclusive` chặn. Modifier giữ làm Down chờ, Up đến hạn đi độc lập. Dùng event hoặc polling thưa có trần sau khi được duyệt. Không thêm state ModifierHold, ghim Up hay đổi timeline khi giữ modifier. Kiểm thử focus/manual chồng nhau, Skip/Stop, mixed Up-only fallback. |
| Đợt 4 · supervisor và retirement | Tách tiến độ native khỏi UI. Giảm còn `Starting`, `Active`, `Paused`, `Draining` cho worker. Quarantine/Retired thuộc shell. Paused với journal sạch không arm timer định kỳ và xác minh lease trước Resume. Guard nhỏ của supervisor báo expiry và signal một lần nếu thoát bất thường hoặc unwind. Guard disarm khi shutdown có chủ đích. Kiểm thử mất process, supervisor/UI stall và residual trước khi bỏ watchdog riêng. |
| Lượt cuối · cleanup tài liệu và context | Làm sau khi hành vi và qualification gate đã chốt. Giao theo cả cây tài liệu và nơi nạp context, dùng danh mục xóa/thu gọn đã được leader review. Nội dung và tiêu chí ở mục 6. |

## 6. Cleanup tài liệu cuối

**Mục tiêu.** Agent đọc cây hiện hành chỉ gặp một hợp đồng cho mỗi chủ đề. Nội dung tham chiếu runtime phải khớp mã ở SHA cuối. Git và PR giữ lịch sử thay cho nhiều bản kế hoạch, report cũ hoặc hướng dẫn gần giống nhau.

| Loại nội dung | Xử lý |
| --- | --- |
| Hợp đồng đang có hiệu lực | Giữ `docs/playback-engine.md` là nơi duy nhất mô tả lifecycle, authority, H/G, cleanup và retirement. Giữ `docs/playback-verification.md` là runbook duy nhất cho Windows. `SECURITY.md` nêu ranh giới. `docs/architecture.md`, `docs/INDEX.md` và hướng dẫn agent dùng liên kết ngắn tới nơi sở hữu. |
| Bản thiết kế cũ, ghi chú giai đoạn, checklist trùng, bảng timing cũ hoặc hướng dẫn agent lặp | Xóa khỏi cây làm việc sau khi đã hòa các quy định còn hiệu lực vào đúng hợp đồng. Ưu tiên xóa thay vì tạo thư mục archive trong repo. Nếu một record có giá trị provenance, thêm tối đa một liên kết immutable SHA ở `docs/HISTORY.md`. |
| Kế hoạch này và bằng chứng riêng từng đợt | Giữ link tới commit kế hoạch và evidence trong PR. Xóa file kế hoạch tạm trong commit cleanup cuối. Không đưa log, trace hoặc số liệu tạm vào active docs. |
| Nội dung thuộc release, ký, updater hoặc bảo mật ngoài playback | Giữ theo hợp đồng sở hữu và scope task. Nếu phát hiện drift, ghi rõ vào review để chủ dự án định phạm vi. |

CLI nộp danh sách **Xóa / Gộp / Giữ** với lý do và hợp đồng đích trước khi áp dụng cleanup. Kiểm tra toàn cây có liên quan, gồm `AGENTS.md`, `docs/AGENTS.md`, `docs/INDEX.md`, `docs/HISTORY.md`, README, PR templates, nhận xét dài trong code và mọi link đến tài liệu dispatch cũ. Sau duyệt, xóa mạnh tay bản trùng và tham chiếu chết. Chạy audit tài liệu và link trong `cargo xtask check static`, kiểm tra `git diff --check`, xác minh lệnh runbook trên SHA đang xét. Nộp số file xóa/gộp và contract còn lại. Leader review commit cleanup ngay khi push. Chủ dự án nhận báo cáo merge readiness riêng cho exact HEAD.

## 7. Mẫu báo cáo của CLI trên draft PR

```text
Đợt và phạm vi
SHA branch và base / build SHA nếu khác
Nhóm thay đổi đã xong và nhánh trùng đã bỏ
Lệnh đã chạy, exit code, số test và phần chưa chạy
Evidence Windows, table A/B/C, artifact và provenance
Kết quả race, fallback, cleanup và giới hạn phép đo
Trạng thái CI tại SHA này
Quyết định cần leader và phần độc lập đang tiếp tục
```

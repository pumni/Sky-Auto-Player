# Kế hoạch triển khai và nghiệm thu bỏ G

**Nhánh:** `refactor/playback-remove-release-gap`  
**Base:** `1d00c3b71fbf1f94003865bad6c4eba6d8dcd902` trên `main`  
**Trạng thái ban đầu:** giao đợt 1. Những đợt sau có phạm vi bên dưới và được giao trong PR khi đã có kết quả liên quan.

## Mục tiêu và các quyết định đã chốt

- Bỏ **G**, sàn chờ cố định sau Up, trong đường phát nhạc. Ở máy 60 FPS với hold 1,0 frame và margin 500 µs, giữ **H = 17.167 µs**. Giữ FPS, hold và margin hiện có cho dữ liệu đã lưu. Các cấu hình FPS khác vẫn tính H theo lựa chọn hiện tại. Lịch giữ đúng onset và hợp âm tác giả, trong giới hạn của H và thứ tự phát.
- Một Down kế tiếp cùng phím chỉ đủ điều kiện khi Up của generation trước **đã commit** với bằng chứng transport và QPC hợp lệ, và Down đã tới hạn theo lịch. Khi hai cạnh cùng mốc, worker gửi Up rồi đánh giá lại Down và chạy final admission tươi. Các Up và Down rời phím có thể ghép như hiện tại. Mỗi lần gọi vật lý giữ mask Up/Down rời nhau.
- Khi một cặp lặp hợp lệ có khoảng nhả danh định dưới một tick, cho thấy lý do xác nhận bằng UI có sẵn. Người dùng có thể tiếp tục. Cảnh báo chỉ mô tả khả năng game bỏ qua khoảng Up, không thêm sàn, không tự tăng hold hoặc giảm tempo. Nếu thực sự dưới H, giữ lỗi lịch với vị trí nguồn.
- Worker giữ một writer, journal trước lần gọi, generation accounting, QPC, kiểm foreground/owner/lease/modifier theo loại gói, và quy tắc xử lý transport partial/zero. Bảo toàn cleanup, recovery sau join và tiêu chí xử lý residual. Dùng public Win32 `SendInput` theo [SECURITY.md](../SECURITY.md).
- Bỏ G sau resume là thay đổi vòng đời riêng. Sửa nóng QoS, vị trí admission và chiến lược spin cần kết quả A/B và quyết định của leader. Giữ H gần một frame trong tất cả các phép A/B để so công bằng.
- Đánh giá trên Windows 11, Rust **1.99.1**, target **x86_64-pc-windows-msvc** khi có host phù hợp. Thu bằng chứng sender và cửa sổ ReceiveOnly của dự án. Hành vi Sky và tiếng người chơi khác nghe được nằm ngoài phép đo này. FPS màn hình/game và nhịp lấy mẫu input là hai đại lượng có thể khác nhau.

Các hợp đồng sở hữu gồm [playback engine](playback-engine.md), [song planning](song-planning.md), [Windows verification](playback-verification.md), [toolchain policy](rust-toolchain-policy.md) và [release authority](v4-release-authority.md).

## Cách điều phối qua GitHub

| Vai trò | Trách nhiệm |
|---|---|
| Leader | Giao phạm vi trong PR, chốt thiết kế và mọi thay đổi có ảnh hưởng đến semantics. Review diff theo **HEAD SHA** vừa push, kể cả CI đang chạy hoặc đỏ. Ghi phát hiện phải sửa, đánh giá bằng chứng Windows và giao đợt kế tiếp. |
| Implementation agent trên máy Windows | Đọc `AGENTS.md` và kế hoạch, tự đi trọn một đợt đã giao, triển khai và xóa đường cũ liên quan. Chạy format, lint, test có mục tiêu, push lên cùng nhánh và báo kết quả vào chính PR. Có thể tự chọn chi tiết cục bộ giữ nguyên hợp đồng. |
| Chủ repo | Điều phối phiên máy Windows tương tác, quyết định merge/phát hành. Lỗi chạy native hoặc thiếu quyền được báo bằng trạng thái thực tế. |

Mọi code thay đổi nằm trên nhánh thực hiện duy nhất. So A/B với baseline bằng detached checkout/worktree nếu cần. Dùng các commit dễ phân biệt nhóm thay đổi, push theo checkpoint của cả đợt. Giữ lịch sử SHA để lần review có thể tái lập. Dùng commit sửa tiếp khi leader yêu cầu thay đổi.

Implementation agent đưa các quyết định quan trọng lên PR, gồm đổi timing window/target authority, giải quyết lỗi bằng cách giảm fail-closed, thay cấu trúc/dữ liệu công khai ngoài phạm vi đã chốt, chính sách giữ/nhả/resume, đổi ngưỡng qualification, dependency/toolchain, retirement calibration, và thay đổi installer/signing/release. Nêu bằng chứng, lựa chọn và ảnh hưởng đến phần đang làm, rồi tiếp tục phần độc lập đã được giao. Giữ các vấn đề thiết kế riêng khỏi sửa cách đặt tên hoặc cấu trúc hàm cục bộ.

### Báo cáo bàn giao mỗi đợt

Đăng một comment trên PR theo HEAD vừa push. Ghi rõ `đợt`, `HEAD SHA`, phạm vi đã xong, phần còn mở và lý do, lệnh kiểm thử với exit code/số test, bằng chứng Windows hoặc `NOT RUN`, danh mục code đã xóa, và các việc cần leader quyết định. Link tới log/artifact đúng SHA, giữ raw trace ngoài tree nếu có. Chỉ dùng dữ liệu được phép công khai trong PR. Dùng brief review tiếp theo để sửa theo finding, giữ mọi trao đổi trên PR này.

Leader bắt đầu review khi có commit mới. Kết quả CI bàn riêng với nhận xét mã. Qua review một đợt nghĩa là phạm vi của đợt đạt hợp đồng đã giao. Merge readiness còn theo gate cuối của repo và do chủ repo quyết định.

## Đợt 1 · Bỏ G xuyên suốt từ sheet đến desktop

**Phạm vi giao ngay.** Hoàn tất một luồng ổn định từ planner, compiler, executor, Tauri tới UI và qualification offline. Gỡ cả cấu trúc trùng lặp đi cùng G trong luồng này. Tách commit chuẩn hóa/migration trong cùng nhánh nếu giúp review, với một checkpoint cho tổng đợt.

- **Planner.** Chỉ từ chối repeat thực sự không đủ H trong miền µs. Up theo `down + H`, giữ phép cộng kiểm overflow và quy tắc dedup. Gỡ nhánh nén hold bất khả đạt do hold mục tiêu bằng sàn, đồng thời xóa cảnh báo/thống kê chỉ phục vụ nhánh đó. Giữ khoảng Up tự nhiên như số đo. Đề xuất tempo của lỗi dưới H dựa trên H và xác thực lại variant khi Prepare.
- **Compiler và dữ liệu chuẩn.** Dùng lượt duyệt Up-trước-Down tại cùng timestamp để đóng generation cũ, ghi paired Up vào Down tương ứng rồi mở generation mới. Không còn lỗi chỉ vì một Up hợp lệ và Down kế tiếp cùng phím có cùng mốc. Giữ kiểm source order, timestamp, phím allowlist, duplicate, overlap generation, Down không có Up, checked size và cancellation. Giữ source ID, reason ID còn consumer và metadata Up không ghép. Up không ghép vẫn không phát vật lý. Với source Down được liệt kê trước Up ở cùng mốc, phải giữ ID gốc và thực thi đúng thứ tự quan hệ generation.
- **Rút gọn arena.** Di chuyển validation, ngân sách `timing_advisory`, startup metrics, kiểm thử và acceptance export sang dữ liệu Down đã có paired Up cùng bộ đếm/metadata tối thiểu. Tách `authored group count` và số lần gọi vật lý. Nếu report cần các cạnh authored, dựng projection tại adapter xuất báo cáo, version hóa khi đổi nghĩa, tránh duy trì lại packet arena trong production. Sau khi chuyển consumer, xóa `CompiledPacket`, `RuntimeSchedule::packets` và phần `CompiledBatch/CompactIntent` đã hết đường dùng, kể cả view/re-export/helper cũ. Chuyển các test độc lập có ích của `analyze_whole_stream*` sang mô hình dùng executor rồi xóa đường lập lịch song song. Nếu phát hiện consumer được hỗ trợ mà phần thay thế chưa đáp ứng được, nêu call site và đưa leader quyết định trước khi giữ API tương thích.
- **QPC.** Kiểm lịch authored trong µs trước khi quy đổi. Chuẩn hóa Up trong prepared arena bằng `max(ceil(authored_up), ceil(authored_down) + ceil(H))` với checked arithmetic. Giữ mốc authored phục vụ provenance/hạch toán. H lúc chạy vẫn tính theo Down completion. Thử QPC 3.579.545 Hz với Down 1.000 µs, H 17.167 µs, Up chuẩn hóa từ 65.030 thành 65.031 ticks. Không thêm một tick vào Down như một sàn release ngầm.
- **Runtime.** Bỏ state `next_down_not_before_qpc` từng slot, G floor trên musical Down, và mọi đường tính `Up_due + G`. Khi chord vướng phím active, đánh thức theo Up đủ điều kiện gần nhất, gửi Up và re-evaluate sau commit. Trường hợp trùng deadline phải tiến triển và không lặp wait tại hiện tại. Không thay đổi batching chord rời phím, lỗi partial/zero hoặc cleanup khi bỏ G. Giữ hiệu ứng của thời gian chờ **sau verified cleanup** trong đợt 1 bằng policy có tên và phạm vi resume riêng, tính như baseline hiện tại. Code này được thay trong đợt 2, không dùng để chờ repeat.
- **Desktop và evidence contracts.** Lấy số cặp có `0 <= gap < frame_us` từ lịch QPC-ready hoặc mô hình authored tương ứng để đưa vào `RiskReport`. Với lý do này, mức tối thiểu là medium cho trường hợp phát vật lý khả thi. Từ `PlaybackAdmission`, hiện lý do trước lần gửi đầu tiên và giữ lựa chọn tiếp tục. Bài không có rủi ro mới vẫn theo cơ chế Play hiện tại. Bỏ ngưỡng G, G-floor fields và chữ H+G ở DTO, binding, UI, mock và trace/report hiện hành. Giữ số đo gap nếu có consumer. Version hóa schema đổi nghĩa, fingerprint mới cho Prepare, giữ dữ liệu nhạc và settings cũ. Sửa fixture PowerShell và acceptance kiểm sàn G thành kiểm thứ tự cạnh/H và đủ generation. Không gắn fixture lịch sử vào semantics mới.
- **Đơn giản hóa Prepare có thể chứng minh.** Thu Up không ghép ngay khi compile, tránh hai lượt quét arena cũ. Đổi đếm mật độ theo cửa sổ trượt, giữ riêng số mốc Down và tổng số phím. Bỏ công thức byte dành riêng cho cấu trúc đã xóa. Với allocation còn tồn tại, giữ kiểm ngân sách cho input đã mở rộng, reason bytes, plan/variant đồng thời và ownership trong khi hủy. Dùng checked size/capacity và `try_reserve` cho đường mới.

### Nghiệm thu đợt 1

- `Δ = H−1, H, H+1 µs` ở planner/compiler, các repeat 20, 25,5, 33, 34,334 và 40 ms. Thử cùng timestamp có thứ tự source đảo, chord có một phím repeat, disjoint mixed, unmatched Up, overflow, cancel và input lớn. Một Up trễ được dùng để kiểm cặp trước đây đủ G trên lịch nhưng gap đầu nhận có thể co.
- Deterministic worker kiểm Up commit trước Down, H từ Down completion, các trường hợp partial/zero/clock anomaly, không retry Down đã thử và no allocation khi healthy. Chèn Stop, Pause, suspend, focus loss, modifier và owner loss đúng giữa Up và Down.
- Tauri/React kiểm plan có gap ngắn hiển thị lý do trước Start và vẫn Proceed, plan ready đi tự động, các variant và fingerprint không lệch, settings cũ đọc được và binding đã regenerate. Kiểm số lượng, nghĩa và nguồn của các metric đã thay.
- Báo cáo so sánh dung lượng Prepare quanh peak và các arena đã bỏ với workload hiện có. Rà bằng `rg` toàn repo cho G floor, type cũ, API cũ, re-export, test helper/feature, DTO, script và tài liệu liên quan. Mọi match còn lại phải là số đo đang dùng, hàm resume riêng cho đợt 2 hoặc dữ liệu lịch sử có lý do rõ. Xóa code không còn caller và cờ tạm, tránh `allow(dead_code)` để che phần chưa dọn.
- Dùng fixture tổng hợp cho vùng chênh. Phép quét tại base đã thấy **94 bản legacy, 74.102 cặp, khoảng ngắn nhất 76 ms** ở tempo 1,00×. Danh mục đó hữu ích cho hồi quy, còn lịch biên và một bản import phù hợp mới bộc lộ khả năng mở khóa.

## Đợt 2 · Resume, recovery và đo đường gửi Windows

**Phạm vi giao sau review đợt 1.** Hoàn tất vòng đời sau khi bỏ thời gian chờ số học cuối cùng và tạo evidence để leader quyết định hiệu năng.

- Gỡ `cleanup_completed_qpc + G` và policy chờ resume chuyển tiếp. Điều kiện resume dùng verified cleanup, journal sạch, generation/owner/lease/foreground còn hợp lệ và sự kiện resume đã commit. Giữ chứng cứ clock cần cho epoch. Kiểm manual Pause→Resume rất nhanh và focus đã ổn định đủ grace **trước khi** cleanup mới gửi Up. `GetAsyncKeyState` chỉ cung cấp dấu hiệu trạng thái Windows và có thể trả 0 khi truy vấn thất bại. Grace hay thao tác người dùng không xác nhận game đã lấy mẫu Up.
- Dọn đoạn lặp cleanup/recovery quanh residual nếu dùng chung đúng semantics. Giữ `needs_up` và `needs_probe`, giới hạn drain, ownership một writer, rào join, quy tắc origin foreground và cách chặn phiên vật lý mới khi residual còn mơ hồ. Hiện lý do và trạng thái retry rõ trong các bề mặt hiện có. Một lời gọi `KEYUP` cho `active_mask` không thay thế được journal.
- Đi hết ca test resume, flow Stop→release→next Start, focus/owner switch, retry, quarantine và Stop/lease/suspend giữa hai lượt Up/Down. Rà và xóa các fixture/helpers chỉ còn nói về timer resume cũ. Giữ evidence cũ pin theo schema/binary.
- Trên Windows, chạy baseline/candidate bằng cùng Rust, target, profile `dist`, workload, nguồn điện và cấu hình 60 FPS/H. Ghi đủ `wake → gate_start/end → send_begin/end → commit` ở chuỗi sát H, đo `Up_end → Down_start`, p50/p95/p99, số mẫu, CPU/spin và Stop→release. ReceiveOnly ghi cả cạnh. Chạy mô hình poll 60 Hz nhiều pha từ log, thêm khoảng 18/20 ms và tick bị bỏ qua như phân tích độ nhạy. Báo riêng triplet Down→Up→Down theo từng slot, các cặp mới hợp lệ và các cặp cũ bị co thời gian nhả.
- Dùng phép thử hiện có để so startup calibration trước/sau guards, fixed spin thử 500 µs và từng bit process QoS, chỉ đổi một yếu tố mỗi lần. Có Quiet/Contention, ghi điều kiện occluded hay foreground. Đường production dùng high-resolution timer riêng, nên `timeBeginPeriod` fallback và QoS là các điều kiện cần đo độc lập. Đo mỗi gate, giữ tươi authority khi thử đưa staging và phép tính thuần sớm hơn. Giữ chính sách production hiện tại ở những arm chưa được leader chấp nhận.

Không lấy CI, mock, PlanOnly hay cửa sổ ReceiveOnly làm bằng chứng Sky đã phát âm. Nếu Windows native chưa chạy được, gửi phần review diện mã và mock đã có, ghi `NOT RUN` cho phần native với lý do.

## Đợt 3 · Thay đường calibration riêng, thu gọn telemetry và chốt kiểm chứng

**Phạm vi sau khi leader chốt từ đợt 2.** Làm trọn luồng diagnostics đến installer trong cùng nhánh.

- Chỉ đưa biến thể wake/admission/QoS đã được leader chốt vào production. Xóa nhánh thử, counters và option hết vai trò. Giữ check foreground/owner tươi, control mới, journal và QPC. Nếu lợi ích wake không vững, giữ policy baseline và dọn các arm không cần thiết trong scope.
- Dùng phép đo sender đến từ đường production cho diagnostics, giữ margin thủ công của người dùng. Bỏ nhánh Raw Input legacy rồi retire `native_calibration.exe`, đường đọc cache `input_latency.json` và phần UI/IPC/schema đã được thay, theo quyết định leader dựa trên parity. Không xóa cache trên đĩa người dùng chỉ để dọn. Sửa cùng lúc configure Tauri bundle, CI manifest, install/upgrade, xtask verification và các báo cáo liên quan. Giữ signing, updater, release custody và provenance. Đợt này sửa code/hợp đồng đóng gói trong nhánh, không chạy publish/deploy.
- Thu gọn snapshot theo danh mục consumer và dữ liệu A/B. Counter HUD độc lập có thể cập nhật nhẹ, các trường liên quan và terminal result giữ snapshot nhất quán của cùng session. Bỏ trace fields, buffer, subscriptions và adapter đã hết consumer, kể cả test cũ chỉ giữ chúng sống. Chốt lại budget Prepare theo arena mới và đo workload biên trước khi xóa mọi dự đoán dung lượng cũ còn thay thế được.
- Chuẩn bị lần review cuối theo exact HEAD. Bản kết quả nêu rõ thứ tự cạnh, invariants qua fault tests, số trường hợp gap bị poller mô hình bỏ lỡ, thống kê trước/sau, số test, các đường đã xóa và mọi giới hạn. Chạy các gate repo ở đúng SHA. Bản kế hoạch là work order của PR, hợp đồng sản phẩm sau triển khai nằm trong các tài liệu sở hữu hiện có.

## Lệnh và báo cáo kiểm thử

Chạy ở repo root Windows với công cụ pin của repo. Khi sửa frontend, thêm `bun run check` ở `apps/desktop` và test mục tiêu theo scripts hiện hành. Regenerate binding khi DTO đổi. Dùng runbook để chạy qualification tương tác và xuất bằng chứng đúng SHA.

```powershell
node scripts/toolchain-pin.mjs exec -- cargo fmt --manifest-path rust/Cargo.toml --all -- --check
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path rust/Cargo.toml -p sky_app_core -p sky_dispatch_core
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc -- --test-threads=1
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features
node scripts/toolchain-pin.mjs exec -- cargo xtask bindings generate
node scripts/toolchain-pin.mjs exec -- cargo xtask bindings check
node scripts/toolchain-pin.mjs exec -- cargo xtask check rust
pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
pwsh -NoProfile -File scripts/test_evidence_contracts.ps1
git diff --check
```

Khi đã sẵn sàng merge, dùng `node scripts/toolchain-pin.mjs exec -- cargo xtask check all` và đối chiếu required CI trên đúng HEAD theo `AGENTS.md`. Format, lint và local test là bằng chứng triển khai. CI chưa xong vẫn có thể review diff và yêu cầu sửa. Giữ trạng thái chính xác cho các phép thử không chạy hoặc chưa đủ điều kiện.

import type { HomeContent } from './home.types';

export const homeVi: HomeContent = {
  locale: 'vi',
  seo: {
    title: 'Sky Auto Player — Trình phát nhạc Sky tự động trên Windows',
    description:
      'Trình phát sheet nhạc cho Sky trên Windows 11 64-bit. Ứng dụng lên lịch sự kiện từ sheet và gửi đầu vào qua Windows SendInput khi Sky được xác minh là cửa sổ đang ở foreground.',
  },
  navigation: {
    overview: 'Tổng quan',
    howItWorks: 'Cách hoạt động',
    guides: 'Hướng dẫn',
    faq: 'FAQ',
    github: 'GitHub',
    release: 'Bản phát hành',
  },
  hero: {
    kicker: 'Sky Auto Player cho Sky: Children of the Light · Windows 11 64-bit',
    titleLines: ['Chơi bản nhạc,', 'không chơi bàn phím.'],
    description:
      'Nạp sheet và chọn bài hát. Sky Auto Player lên lịch đầu vào cho nốt, hợp âm và nốt ngân theo dữ liệu timing, rồi gửi qua Windows SendInput khi Sky là target foreground đã được xác minh; game tự quyết định thời điểm lấy mẫu đầu vào.',
    primaryCta: 'Tải bộ cài Windows',
    primaryCtaDestination: 'Mở GitHub Releases',
    secondaryCta: 'Xem cách hoạt động',
    metadata: [
      'JSON',
      'SKYSHEET',
      'TXT',
      'MIỄN PHÍ · MÃ NGUỒN MỞ',
      'WINDOWS 11 64-BIT',
      'TAURI DESKTOP',
    ],
    riskNote: 'Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.',
    riskNoteLink: 'Hãy dùng có trách nhiệm và tự chịu rủi ro.',
    affiliationDisclaimer:
      'Dự án cộng đồng không chính thức. Không có liên kết hay sự chứng thực từ thatgamecompany.',
  },
  proofStrip: {
    kicker: 'Mô hình phát nhạc',
    metric: 'SỰ KIỆN SHEET / SENDINPUT',
    signals: [
      'Các phím trong hợp âm được gửi trong cùng một batch SendInput',
      'Adaptive lead theo dõi độ trễ hoàn tất phía gửi',
      'Timing nốt ngân theo sheet và thiết lập hold frame',
      'HUD hiển thị số đo phía gửi, không phản ánh sampling phía game',
    ],
  },
  playback: {
    kicker: 'Được xây dựng quanh bản nhạc',
    title: 'Timing cũng là một phần của nhạc cụ.',
    description:
      'Sky Auto Player lên lịch sự kiện theo mốc thời gian phía gửi rồi đo độ trễ hoàn tất để điều chỉnh adaptive lead. Windows và vòng lặp lấy mẫu đầu vào riêng của Sky quyết định khi nào game đăng ký phím; ứng dụng không thể bảo đảm thời điểm phát âm thanh hay frame trong game.',
    points: [
      'Các phím trong hợp âm gửi cùng một batch SendInput',
      'Timing sheet có thiết lập tempo riêng cho mỗi bài',
      'Sự kiện nốt, hợp âm và nốt ngân',
      'Adaptive lead dựa trên số đo hoàn tất phía gửi',
      'HUD hiển thị số đo timing phía gửi',
      'Xem thử bằng dry-run',
    ],
  },
  comparison: {
    kicker: 'Từ sự kiện sheet đến đầu vào',
    title: 'Phát nhạc theo dữ liệu sheet.',
    sequenceHeader: 'Chuỗi độ trễ cố định',
    playerHeader: 'Sky Auto Player',
    rows: [
      { sequence: 'Chuỗi lần nhấn phím', player: 'Sự kiện nốt, hợp âm và nốt ngân theo sheet' },
      {
        sequence: 'Một mẫu khoảng cách cố định',
        player: 'Timing theo dấu thời gian của từng sự kiện',
      },
      { sequence: 'Một tempo dùng cho mọi bài', player: 'Thiết lập tempo được lưu theo bài' },
      { sequence: 'Một trường độ hold', player: 'Thiết lập hold frame được lưu theo bài' },
      { sequence: 'Không có số đo phía gửi', player: 'Adaptive lead dùng số đo hoàn tất phía gửi' },
      {
        sequence: 'Timing chỉ từ chuỗi phím',
        player: 'Diagnostics hiển thị số đo timing phía gửi',
      },
    ],
  },
  product: {
    kicker: 'Ứng dụng thực tế',
    title: 'Thư viện, profile timing và điều khiển trong cùng một giao diện.',
    description:
      'Ứng dụng Windows có thư viện bài hát tích hợp, liked songs, playlist và diagnostics.',
    annotations: [
      'Duyệt bài tích hợp sẵn và liked songs trong Library.',
      'Tạo hoặc chọn playlist, rồi dùng Add songs để nhập file hay cả thư mục.',
      'File được nhập vẫn ở vị trí gốc; ứng dụng không di chuyển hay xóa chúng.',
    ],
  },
  steps: {
    kicker: 'Ba bước',
    title: 'Các bước bắt đầu với sheet.',
    items: [
      {
        title: 'Tải xuống',
        description:
          'Tải installer Tauri NSIS canonical từ trang GitHub Releases chính thức của Sky Auto Player và cài đặt cho current user.',
      },
      {
        title: 'Thêm sheet nhạc',
        description:
          'Tạo hoặc chọn playlist, rồi dùng Add songs để nhập file JSON, .skysheet, TXT tương thích JSON hoặc cả thư mục. File nguồn vẫn ở vị trí ban đầu.',
      },
      {
        title: 'Phát nhạc',
        description:
          'Chọn bài và bắt đầu phát khi Sky đang ở foreground. Ứng dụng kiểm tra target trước khi gửi đầu vào.',
      },
    ],
    workflowNote: 'Nhấn / hoặc Ctrl+F để focus tìm kiếm · Esc đóng overlay an toàn',
  },
  technical: {
    kicker: 'Giới hạn kỹ thuật',
    title: 'Rõ ràng về những gì ứng dụng làm và không làm.',
    description:
      'Sky Auto Player là ứng dụng riêng cho Windows 11. Ứng dụng chỉ gửi đầu vào qua Windows SendInput sau khi kiểm tra target xác nhận Sky đang ở foreground; mã nguồn được công khai để kiểm tra.',
    ledger: [
      { term: 'Đầu vào', definition: 'Windows SendInput', state: 'yes' },
      { term: 'Tiến trình', definition: 'Ứng dụng độc lập', state: 'info' },
      { term: 'Bộ nhớ game', definition: 'Không được đọc/kiểm tra', state: 'no' },
      { term: 'Inject code', definition: 'Không sử dụng', state: 'no' },
      { term: 'File game', definition: 'Không sửa đổi', state: 'no' },
      { term: 'Giấy phép', definition: 'GNU GPL v3.0', state: 'info' },
      {
        term: 'Cập nhật',
        definition: 'Tauri updater xác minh sidecar .exe.sig',
        state: 'info',
      },
    ],
    notice:
      'Điều khoản Dịch vụ: Các giới hạn kỹ thuật trên không bảo đảm an toàn cho tài khoản. Tự động phát nhạc vẫn có thể xung đột với Điều khoản Dịch vụ của Sky. Hãy dùng công cụ có trách nhiệm và tự chịu rủi ro.',
  },
  formats: {
    kicker: 'Sheet được hỗ trợ',
    title: 'Nạp các định dạng cộng đồng đang sử dụng.',
    items: [
      {
        extension: '.json',
        name: 'JSON',
        description:
          'Sheet JSON có cấu trúc, chứa sự kiện âm nhạc và metadata mà trình phát hỗ trợ.',
        tags: 'NỐT · HỢP ÂM · NGÂN',
      },
      {
        extension: '.skysheet',
        name: 'Skysheet',
        description:
          'Sheet dựa trên JSON với phần mở rộng .skysheet từ hệ sinh thái trình soạn nhạc Sky.',
        tags: 'SHEET EXPORT',
      },
      {
        extension: '.txt',
        name: 'TXT tương thích JSON',
        description: 'File văn bản thuần chứa cấu trúc sheet tương thích JSON.',
        tags: 'VĂN BẢN THUẦN',
      },
    ],
  },
  faqPreview: {
    kicker: 'Trước khi tải',
    title: 'Một vài câu trả lời hữu ích.',
    readMoreLink: 'Đọc toàn bộ FAQ',
    items: [
      {
        question: 'Sky Auto Player có miễn phí và mã nguồn mở không?',
        href: '/vi/faq/#free',
      },
      {
        question: 'Hỗ trợ những định dạng sheet nào?',
        href: '/vi/faq/#formats',
      },
      {
        question: 'Việc dùng tool có ảnh hưởng tài khoản Sky không?',
        href: '/vi/faq/#account-safety',
      },
    ],
    guideLinks: [
      { label: 'Sky Auto Player hoạt động như thế nào', href: '/vi/guides/how-it-works/' },
      { label: 'Xử lý sự cố thường gặp', href: '/vi/guides/troubleshooting/' },
    ],
  },
  finalCta: {
    title: 'Màn trình diễn tiếp theo đã nằm sẵn trong sheet.',
    description:
      'Mở bản phát hành chính thức, thêm sheet vào playlist và xem các thiết lập phát hiện có.',
    primaryCta: 'Xem các bản phát hành',
    secondaryCta: 'Xem mã nguồn trên GitHub',
  },
};

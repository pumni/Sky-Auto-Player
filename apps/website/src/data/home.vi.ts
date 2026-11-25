import type { HomeContent } from './home.types';

export const homeVi: HomeContent = {
  locale: 'vi',
  seo: {
    title: 'Sky Auto Player — Trình phát sheet nhạc Sky trên Windows 11',
    description:
      'Trình phát sheet nhạc miễn phí, mã nguồn mở cho Sky trên Windows 11. Dùng sheet của bạn hoặc chọn bài từ thư viện tích hợp.',
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
    kicker: 'Trình phát sheet nhạc miễn phí, mã nguồn mở cho Sky trên Windows 11',
    titleLines: ['Chơi bản nhạc,', 'không chơi bàn phím.'],
    description:
      'Chọn bài từ thư viện tích hợp hoặc dùng sheet của riêng bạn. Tạo playlist, chọn cách bạn muốn chơi, rồi để Sky Auto Player xử lý từng nốt nhạc.',
    primaryCta: 'Tải bộ cài Windows',
    primaryCtaDestination: 'Mở GitHub Releases',
    secondaryCta: 'Xem giao diện',
    affiliationDisclaimer:
      'Dự án cộng đồng không chính thức, không liên kết hay được thatgamecompany chứng thực.',
  },
  benefits: {
    kicker: 'Dành cho cách bạn chơi',
    items: [
      {
        number: '01',
        title: 'Chơi theo sheet',
        description: 'Nốt đơn, hợp âm và nốt ngân theo dữ liệu âm nhạc trong từng sheet.',
      },
      {
        number: '02',
        title: 'Sắp xếp các bài hát',
        description: 'Duyệt thư viện tích hợp, yêu thích bài hát và sắp xếp thành playlist.',
      },
      {
        number: '03',
        title: 'Dùng sheet của bạn',
        description: 'Nhập file được hỗ trợ vào playlist; file gốc vẫn ở vị trí ban đầu.',
      },
      {
        number: '04',
        title: 'Miễn phí, mã nguồn mở',
        description: 'Xem mã nguồn trên GitHub và tải ứng dụng Windows miễn phí.',
      },
    ],
  },
  product: {
    kicker: 'Bên trong trình phát',
    title: 'Nơi lưu bài hát, sheet và thiết lập.',
    description:
      'Chuyển giữa thư viện, chi tiết bài hát và thiết lập phát nhạc. Chọn một ảnh chụp giao diện desktop để xem rõ hơn.',
    selectorLabel: 'Chọn giao diện trình phát',
    openImageLabel: 'Mở ảnh chụp giao diện desktop này ở kích thước đầy đủ',
    fallbackLabel: 'Mở ảnh chụp giao diện desktop',
    captureLabel: 'Ảnh chụp giao diện desktop',
    views: [
      {
        id: 'library',
        label: 'Thư viện',
        alt: 'Ảnh chụp giao diện desktop hiển thị thư viện bài hát và playlist của Sky Auto Player.',
        caption: 'Duyệt bài hát, bài yêu thích và playlist.',
        src: '/assets/images/library-real-tauri.png',
      },
      {
        id: 'song-detail',
        label: 'Chi tiết bài hát',
        alt: 'Ảnh chụp giao diện desktop hiển thị bài hát đã chọn và chi tiết sheet.',
        caption: 'Xem lại bài hát trước khi phát.',
        src: '/assets/images/detail-real-tauri.png',
      },
      {
        id: 'settings',
        label: 'Thiết lập',
        alt: 'Ảnh chụp giao diện desktop hiển thị thiết lập phát nhạc và ứng dụng.',
        caption: 'Chọn thiết lập phát phù hợp với cấu hình của bạn.',
        src: '/assets/images/settings-real-tauri.png',
      },
    ],
  },
  quickStart: {
    kicker: 'Từ tải xuống đến bài hát đầu tiên',
    title: 'Ba bước để bắt đầu chơi nhạc.',
    items: [
      {
        title: 'Cài đặt trình phát',
        description:
          'Tải bộ cài NSIS từ GitHub Releases chính thức. Ứng dụng được cài cho người dùng Windows hiện tại.',
        linkLabel: 'Hướng dẫn cài đặt Windows',
        href: '/vi/guides/windows-setup/',
      },
      {
        title: 'Chọn bài hoặc thêm sheet',
        description:
          'Chọn bài trong thư viện hoặc thêm file JSON, .skysheet hay TXT tương thích JSON vào playlist.',
        linkLabel: 'Hướng dẫn định dạng sheet',
        href: '/vi/guides/sheet-formats/',
      },
      {
        title: 'Phát nhạc khi Sky đang được chọn',
        description:
          'Chọn cửa sổ Sky, chọn bài rồi nhấn Play. Ứng dụng kiểm tra cửa sổ foreground và target Sky đã giữ trước mỗi gói input âm nhạc mới.',
        linkLabel: 'Cách hoạt động của trình phát',
        href: '/vi/guides/how-it-works/',
      },
    ],
  },
  playback: {
    kicker: 'Một đoạn nhạc, từng bước một',
    title: 'Theo dõi giai điệu qua từng nốt.',
    description:
      'Phát đoạn nhạc minh họa, tự chuyển từng sự kiện hoặc phát lại. Danh sách sự kiện, phím được đánh dấu, thời gian đã trôi qua và nốt ngân luôn đồng bộ.',
    eventListLabel: 'Sự kiện phát nhạc',
    eventTypes: { chord: 'Hợp âm', note: 'Nốt đơn', hold: 'Nốt ngân' },
    controlsLabel: 'Điều khiển minh họa phát nhạc',
    play: 'Phát minh họa',
    resume: 'Tiếp tục minh họa',
    pause: 'Tạm dừng minh họa',
    replay: 'Phát lại minh họa',
    cancel: 'Hủy phát',
    previous: 'Sự kiện trước',
    next: 'Sự kiện tiếp theo',
    progressLabel: 'Tiến trình minh họa',
    timelineLabel: 'Dòng thời gian đoạn nhạc minh họa',
    keyboardLabel: 'DÀN PHÍM MINH HỌA',
    initialStatus: 'Sẵn sàng. Hợp âm đầu tiên đang hiển thị; hãy phát hoặc chuyển từng sự kiện.',
    staticExplanation: 'Ví dụ tĩnh gồm một hợp âm, một nốt đơn và một nốt ngân theo thứ tự này.',
    playingStatus: 'Đang phát minh họa',
    pausedStatus: 'Đã tạm dừng minh họa',
    completeStatus: 'Đã phát xong minh họa. Không có phím nào đang hoạt động.',
    manualOnly: 'Thiết bị đang giảm chuyển động. Dùng các nút sự kiện để chuyển qua từng phần.',
    schematicNote:
      'Ví dụ sơ đồ dài 2,6 giây. Các khoảng thời gian chỉ minh họa giao diện, không đại diện cho nhịp của bài hát hay phép đo input trong game.',
  },
  technical: {
    kicker: 'Hai cách mô tả một chuỗi phím',
    title: 'Khoảng cách cố định và sự kiện sheet phục vụ các cách dùng khác nhau.',
    description:
      'Chuỗi độ trễ cố định lặp lại các khoảng thời gian được chọn trước. Sky Auto Player đọc sự kiện nốt đơn, hợp âm và nốt ngân trong sheet rồi lên lịch theo deadline QPC với wake guard cố định đã hiệu chuẩn. Ứng dụng đo thời điểm hoàn tất phía gửi nhưng không liên tục điều chỉnh lead.',
    fixedDelayTitle: 'Chuỗi độ trễ cố định',
    sheetTitle: 'Phát theo sự kiện sheet',
    dimensionHeader: 'Tiêu chí',
    dimensions: [
      {
        label: 'Chuẩn bị gì',
        fixedDelay: 'Chuỗi phím và khoảng cách giữa các lần nhấn',
        sheet: 'Sheet bài hát có các sự kiện âm nhạc',
      },
      {
        label: 'Điều gì lặp lại',
        fixedDelay: 'Mẫu khoảng cách đã chọn',
        sheet: 'Nốt, hợp âm và nốt ngân được ghi trong sheet',
      },
      {
        label: 'Có thể thay đổi gì',
        fixedDelay: 'Chuỗi phím và thời gian chờ',
        sheet: 'Bài hát, tempo và Base Hold trong playback profile',
      },
    ],
    boundaryNote:
      'Lịch chạy Windows và cách Sky lấy mẫu input quyết định lúc game đăng ký phím. Số đo phía gửi không bảo đảm thời điểm phát âm thanh hay frame trong game.',
  },
  formats: {
    kicker: 'Dùng sheet của bạn',
    title: 'Bắt đầu với các định dạng trình phát hỗ trợ.',
    items: [
      {
        extension: '.json',
        name: 'JSON',
        description: 'Sheet bài hát có cấu trúc với sự kiện âm nhạc và metadata được hỗ trợ.',
        tags: 'NỐT · HỢP ÂM · NGÂN',
      },
      {
        extension: '.skysheet',
        name: 'Skysheet',
        description: 'Sheet bài hát dựa trên JSON với phần mở rộng .skysheet.',
        tags: 'FILE SHEET',
      },
      {
        extension: '.txt',
        name: 'TXT tương thích JSON',
        description: 'File văn bản thuần chứa cấu trúc sheet tương thích JSON.',
        tags: 'VĂN BẢN THUẦN',
      },
    ],
  },
  trust: {
    kicker: 'Tải xuống cùng thông tin rõ ràng',
    title: 'Tìm hiểu bộ cài trước khi cài đặt.',
    description:
      'Sky Auto Player là ứng dụng Windows riêng biệt. Hãy xem ghi chú bản phát hành và thông tin bộ cài trên GitHub trước khi cài.',
    downloadLabel: 'Tải bộ cài Windows',
    downloadDestination: 'Mở GitHub Releases',
    releaseLabel: 'Phiên bản đã phát hành',
    releaseValue: 'v',
    packageLabel: 'Gói cài đặt',
    packageValue: 'Windows 11 64-bit · bộ cài NSIS · cài cho người dùng hiện tại',
    signingLabel: 'Authenticode',
    signingValue:
      'Bộ cài được chủ động phát hành không chữ ký; Windows có thể hiển thị Unknown Publisher hoặc SmartScreen.',
    updaterLabel: 'Cập nhật ứng dụng',
    updaterValue: 'Tauri updater xác minh riêng sidecar .exe.sig.',
    sourceLabel: 'Đọc mã nguồn trên GitHub',
    safetyNote:
      'Ứng dụng không kiểm tra bộ nhớ game, sửa file game, inject code, cài input hook hoặc gắn debugger. Các giới hạn này không bảo đảm an toàn tài khoản; tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.',
    safetyLink: 'Đọc chi tiết an toàn',
  },
  faqPreview: {
    kicker: 'Một vài điều nên biết',
    title: 'Câu hỏi trước khi bắt đầu.',
    readMoreLink: 'Đọc toàn bộ FAQ',
    items: [
      { question: 'Sky Auto Player có miễn phí và mã nguồn mở không?', href: '/vi/faq/#free' },
      { question: 'Có thể dùng định dạng sheet nào?', href: '/vi/faq/#formats' },
      {
        question: 'Tự động phát nhạc có thể ảnh hưởng tài khoản không?',
        href: '/vi/faq/#account-safety',
      },
    ],
    guideLinks: [
      { label: 'Cách hoạt động của trình phát', href: '/vi/guides/how-it-works/' },
      { label: 'Khắc phục sự cố', href: '/vi/guides/troubleshooting/' },
    ],
  },
  finalCta: {
    title: 'Mang một bài nhạc vào Sky.',
    description: 'Khám phá trình phát, chọn sheet và bắt đầu với bản Windows chính thức.',
    primaryCta: 'Tải bộ cài Windows',
    primaryDestination: 'Mở GitHub Releases',
    secondaryCta: 'Xem mã nguồn',
  },
};

import type { HomeContent } from './home.types';

export const homeVi: HomeContent = {
  locale: 'vi',
  downloadCta: 'Tải cho Windows',
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
    release: 'Tải xuống',
  },
  hero: {
    titleStart: 'Mang ',
    titleEmphasis: 'âm nhạc',
    titleEnd: ' đến thế giới Sky.',
    description:
      'Tự động phát sheet nhạc trong Sky: Children of the Light trên Windows. Chọn bài từ thư viện có sẵn hoặc nhập bản nhạc của bạn.',
    secondaryCta: 'Khám phá ứng dụng',
    meta: 'Windows 11 64-bit · Miễn phí, mã nguồn mở · GitHub Releases',
    riskNote:
      'Dự án không chính thức. Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.',
  },
  product: {
    title: 'Khám phá trình phát',
    description: 'Tìm bản nhạc và chuẩn bị trước khi phát.',
    selectorLabel: 'Chọn giao diện trình phát',
    fullSizeLabel: 'Xem ảnh kích thước đầy đủ',
    fallbackLabel: 'Mở giao diện trình phát khác',
    pendingMessage: 'Đang tải ảnh xem trước…',
    errorMessage: 'Không tải được ảnh xem trước. Hãy thử lại hoặc mở ảnh trực tiếp.',
    captureNote:
      'Giao diện minh họa · Dữ liệu mẫu · Mô phỏng trong trình duyệt, không phải ảnh chụp ứng dụng',
    features: [
      {
        title: 'Dùng bản nhạc của riêng bạn',
        description:
          'Nhập tệp hoặc thư mục chứa sheet nhạc tương thích vào danh sách phát. Các tệp gốc vẫn ở nguyên vị trí.',
      },
      {
        title: 'Kiểm tra trước khi phát',
        description:
          'Xem thông tin tương thích, chọn track hoặc cao độ khi được hỗ trợ và xem những phần có thể bị mất khi chuyển đổi sheet nhạc.',
      },
    ],
    views: [
      {
        label: 'Thư viện',
        alt: 'Ảnh xem trước bằng browser mock của thư viện bài hát desktop với bài hát mẫu và thanh bên danh sách phát.',
        caption: 'Tìm bài trong thư viện, lưu bài yêu thích và sắp xếp danh sách phát.',
        src: '/assets/images/desktop-library-browser-preview-1366x768.png',
      },
      {
        label: 'Chi tiết bài',
        alt: 'Ảnh xem trước bằng browser mock của bài hát mẫu đã chọn và chi tiết sheet trên giao diện desktop.',
        caption: 'Kiểm tra khả năng tương thích trước khi phát. Ảnh minh họa đang chọn Blue Bird.',
        src: '/assets/images/desktop-song-detail-browser-preview-1366x768.png',
      },
      {
        label: 'Thiết lập phát',
        alt: 'Ảnh xem trước bằng browser mock của thiết lập phát nhạc desktop với dữ liệu mẫu.',
        caption: 'Điều chỉnh tùy chọn mặc định và thời gian giữ phím.',
        src: '/assets/images/desktop-settings-playback-browser-preview-1366x768.png',
      },
    ],
  },
  playback: {
    title: 'Phát nhạc theo nhịp của bản nhạc',
    description:
      'Trình phát lên lịch cho nốt đơn, hợp âm và nốt ngân theo sheet nhạc. Bạn có thể điều chỉnh tốc độ và thời gian giữ phím.',
    dryRunNote:
      'Kiểm tra thiết lập trước khi chơi. Trong ứng dụng desktop, Test playback (no input) cho phép chạy thử mà không gửi phím.',
    eventTypes: { chord: 'Hợp âm', note: 'Nốt đơn', hold: 'Nốt ngân' },
    controlsLabel: 'Điều khiển minh họa phát nhạc',
    play: 'Chạy minh họa',
    resume: 'Tiếp tục',
    pause: 'Tạm dừng',
    replay: 'Phát lại',
    previous: 'Sự kiện trước',
    next: 'Sự kiện tiếp theo',
    progressLabel: 'Tiến trình đoạn nhạc',
    keyboardLabel: 'Phím',
    initialStatus: 'Sẵn sàng. Đoạn nhạc gồm một hợp âm, một nốt đơn và một nốt ngân.',
    playingStatus: 'Đang phát',
    pausedStatus: 'Đã tạm dừng',
    completeStatus: 'Đã phát xong. Không có phím nào đang hoạt động.',
    manualOnly: 'Đang bật giảm chuyển động. Dùng nút Trước và Tiếp để chuyển từng phần.',
    demoNote:
      'Chạy minh họa để xem hợp âm, nốt đơn và nốt ngân. Không có âm thanh và không đo thời điểm gửi phím trong game.',
    timingLinkLabel: 'Cách tính thời gian phát',
    secondsAbbreviation: 'giây',
  },
  quickStart: {
    title: 'Bắt đầu chơi nhạc trong 3 bước',
    items: [
      {
        title: 'Cài đặt ứng dụng',
        description: 'Tải bộ cài Windows ổn định từ GitHub Releases chính thức.',
      },
      {
        title: 'Chọn bản nhạc',
        description: 'Chọn bài từ thư viện có sẵn hoặc nhập sheet nhạc tương thích.',
      },
      {
        title: 'Phát nhạc trong Sky',
        description:
          'Mở nhạc cụ 15 phím tương thích trong Sky. Chọn bài, nhấn Play trong ứng dụng rồi đưa tiêu điểm về cửa sổ Sky.',
      },
    ],
    setupGuideLabel: 'Đọc hướng dẫn cài đặt',
    setupGuideHref: '/vi/guides/windows-setup/',
    formatsLabel: 'Định dạng',
    formatsValue: '.json · .skysheet · .txt tương thích JSON',
    formatsLinkLabel: 'Định dạng được hỗ trợ',
    formatsHref: '/vi/guides/sheet-formats/',
  },
  trust: {
    title: 'Tải Sky Auto Player',
    platformSummary: 'Windows 11 64-bit · Miễn phí, mã nguồn mở',
    installationTitle: 'Cài đặt',
    installerGuidance: 'Chọn bộ cài Windows ổn định có tên kết thúc bằng _x64-setup.exe.',
    signatureNote: 'Tệp .exe.sig dành cho trình cập nhật, không cần khi cài thủ công.',
    installScope: 'Cài đặt cho tài khoản Windows hiện tại.',
    signingValue:
      'Bộ cài không có chữ ký Authenticode; Windows có thể hiển thị Unknown Publisher hoặc SmartScreen.',
    safetyNote:
      'Ứng dụng gửi phím qua Windows SendInput. Ứng dụng không đọc bộ nhớ game hay sửa tệp game; điều này không bảo đảm an toàn tài khoản.',
    safetyTitle: 'An toàn tài khoản',
    safetyLink: 'Đọc về rủi ro tài khoản',
    supportLabel: 'Trợ giúp',
    supportLinks: [
      { label: 'Cài đặt', href: '/vi/guides/windows-setup/' },
      { label: 'FAQ', href: '/vi/faq/' },
      { label: 'Khắc phục sự cố', href: '/vi/guides/troubleshooting/' },
    ],
  },
};

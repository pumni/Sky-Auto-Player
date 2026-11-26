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
      'Phát nhạc trong Sky: Children of the Light bằng sheet nhạc. Khám phá thư viện bài hát tích hợp hoặc nhập sheet của bạn, rồi chuẩn bị phát nhạc ngay trong ứng dụng desktop.',
    secondaryCta: 'Khám phá ứng dụng',
    meta: 'Windows 11 64-bit · Miễn phí, mã nguồn mở · GitHub Releases',
    riskNote:
      'Dự án không chính thức. Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.',
  },
  product: {
    title: 'Âm nhạc theo cách của bạn.',
    description: 'Tìm bài hát, sắp xếp bộ sưu tập và chuẩn bị phát nhạc.',
    selectorLabel: 'Chọn giao diện trình phát',
    fullSizeLabel: 'Xem ảnh kích thước đầy đủ',
    fallbackLabel: 'Mở giao diện trình phát khác',
    pendingMessage: 'Đang tải ảnh xem trước…',
    errorMessage: 'Không tải được ảnh xem trước. Hãy thử lại hoặc mở ảnh trực tiếp.',
    captureNote:
      'Ảnh xem trước giao diện desktop với dữ liệu mẫu (browser mock, không phải ảnh chụp ứng dụng native).',
    views: [
      {
        label: 'Thư viện',
        description: 'Khám phá bài hát có sẵn, mục yêu thích và playlist.',
        alt: 'Ảnh xem trước bằng browser mock của thư viện bài hát desktop với bài hát mẫu và thanh bên playlist.',
        caption: 'Bài hát có sẵn trong thư viện mẫu.',
        src: '/assets/images/desktop-library-browser-preview-1366x768.png',
      },
      {
        label: 'Chi tiết bài hát',
        description: 'Xem thông tin sheet trước khi phát.',
        alt: 'Ảnh xem trước bằng browser mock của bài hát mẫu đã chọn và chi tiết sheet trên giao diện desktop.',
        caption: 'Bài Blue Bird được chọn và mục Details đang mở trong Inspector.',
        src: '/assets/images/desktop-song-detail-browser-preview-1366x768.png',
      },
      {
        label: 'Thiết lập phát',
        description: 'Điều chỉnh tùy chọn phát phù hợp với thiết lập của bạn.',
        alt: 'Ảnh xem trước bằng browser mock của thiết lập phát nhạc desktop với dữ liệu mẫu.',
        caption: 'Tùy chọn mặc định và timing trong mục thiết lập phát.',
        src: '/assets/images/desktop-settings-playback-browser-preview-1366x768.png',
      },
    ],
  },
  playback: {
    title: 'Không chỉ là một chuỗi phím.',
    description: 'Xem cách một bản nhạc biểu diễn hợp âm, nốt đơn và nốt ngân theo thời gian.',
    eventTypes: { chord: 'Hợp âm', note: 'Nốt', hold: 'Ngân' },
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
    schematicNote:
      'Minh họa trực quan · 2,6 giây · Không phát âm thanh hay đo thời điểm input trong game.',
    timingLinkLabel: 'Cách tính timing',
    secondsAbbreviation: 'giây',
  },
  quickStart: {
    title: 'Từ bản nhạc đến lúc phát.',
    items: [
      {
        title: 'Cài đặt Sky Auto Player',
        description: 'Tải bộ cài Windows ổn định từ GitHub Releases chính thức.',
      },
      { title: 'Chọn bản nhạc', description: 'Duyệt thư viện hoặc nhập sheet tương thích.' },
      {
        title: 'Chuẩn bị trong Sky',
        description: 'Mở nhạc cụ phù hợp, chọn cửa sổ Sky và làm theo hướng dẫn phát nhạc.',
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
    title: 'Sẵn sàng mang âm nhạc đến Sky?',
    description: 'Tải bản phát hành ổn định cho Windows và bắt đầu với bộ sưu tập nhạc của bạn.',
    installerGuidance:
      'Trên GitHub Releases, chọn bộ cài Windows 64-bit của bản ổn định có tên kết thúc bằng _x64-setup.exe. Tệp .exe.sig dành cho trình cập nhật, không cần khi cài thủ công.',
    packageValue: 'Windows 11 64-bit · Miễn phí, mã nguồn mở · cài cho người dùng hiện tại',
    signingValue:
      'Bộ cài không có chữ ký Authenticode; Windows có thể hiển thị Unknown Publisher hoặc SmartScreen.',
    safetyNote:
      'Ứng dụng gửi phím qua Windows SendInput. Ứng dụng không đọc bộ nhớ game hay sửa tệp game; điều này không bảo đảm an toàn tài khoản.',
    safetyLink: 'An toàn tài khoản',
    supportLabel: 'Trợ giúp',
    supportLinks: [
      { label: 'Cài đặt', href: '/vi/guides/windows-setup/' },
      { label: 'FAQ', href: '/vi/faq/' },
      { label: 'Khắc phục sự cố', href: '/vi/guides/troubleshooting/' },
    ],
  },
};

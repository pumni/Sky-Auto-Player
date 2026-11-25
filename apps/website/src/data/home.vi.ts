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
    release: 'Bản phát hành',
  },
  hero: {
    title: 'Chơi bản nhạc, không chơi bàn phím.',
    description: 'Phát sheet nhạc trong Sky từ thư viện tích hợp hoặc sheet của riêng bạn.',
    secondaryCta: 'Xem giao diện',
    meta: 'Windows 11 64-bit · Miễn phí, mã nguồn mở · GitHub Releases',
    riskNote:
      'Dự án không chính thức. Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.',
  },
  product: {
    sectionLabel: 'Giao diện desktop Sky Auto Player',
    selectorLabel: 'Chọn giao diện trình phát',
    fullSizeLabel: 'Xem ảnh kích thước đầy đủ',
    fallbackLabel: 'Mở giao diện trình phát khác',
    views: [
      {
        label: 'Thư viện',
        alt: 'Ảnh chụp giao diện desktop hiển thị thư viện bài hát và playlist của Sky Auto Player.',
        caption: 'Duyệt bài hát, mục yêu thích và playlist.',
        src: '/assets/images/library-real-tauri.png',
      },
      {
        label: 'Chi tiết bài hát',
        alt: 'Ảnh chụp giao diện desktop hiển thị bài hát đã chọn và chi tiết sheet.',
        caption: 'Xem lại bài hát trước khi phát.',
        src: '/assets/images/detail-real-tauri.png',
      },
      {
        label: 'Thiết lập',
        alt: 'Ảnh chụp giao diện desktop hiển thị thiết lập phát nhạc và ứng dụng.',
        caption: 'Chọn thiết lập phát phù hợp.',
        src: '/assets/images/settings-real-tauri.png',
      },
    ],
  },
  playback: {
    title: 'Nhịp điệu là cốt lõi.',
    description: 'Xem cách hợp âm, nốt đơn và nốt ngân đi theo bản nhạc.',
    eventTypes: { chord: 'Hợp âm', note: 'Nốt', hold: 'Ngân' },
    controlsLabel: 'Điều khiển minh họa phát nhạc',
    play: 'Phát đoạn nhạc',
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
    schematicNote: 'Chỉ là minh họa · 2,6 giây · Không đo thời điểm input trong game.',
    timingLinkLabel: 'Cách tính timing',
    secondsAbbreviation: 'giây',
  },
  quickStart: {
    title: 'Bắt đầu trong 3 bước.',
    items: [
      {
        title: 'Cài đặt',
        description: 'Tải bộ cài Windows từ GitHub Releases.',
      },
      {
        title: 'Chọn nhạc',
        description: 'Chọn bài hoặc nhập sheet của bạn.',
      },
      {
        title: 'Phát nhạc',
        description: 'Chọn cửa sổ Sky rồi dùng Play theo hướng dẫn.',
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
    title: 'Sẵn sàng chơi nhạc?',
    packageLabel: 'Bộ cài',
    packageValue: 'Windows 11 64-bit · NSIS · cài cho người dùng hiện tại',
    signingValue:
      'Bộ cài không có chữ ký Authenticode; Windows có thể hiển thị Unknown Publisher hoặc SmartScreen.',
    updaterValue: 'Trình cập nhật Tauri xác minh riêng tệp .exe.sig.',
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

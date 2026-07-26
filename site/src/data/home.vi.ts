import type { HomeContent } from './home.types';

export const homeVi: HomeContent = {
  locale: 'vi',
  seo: {
    title: 'Sky Auto Player — Trình phát nhạc Sky tự động trên Windows',
    description: 'Nạp sheet nhạc Sky và phát nốt, hợp âm cùng nốt ngân đúng thời điểm bằng ứng dụng Windows mã nguồn mở.',
  },
  navigation: {
    playback: 'Phát nhạc',
    howItWorks: 'Cách hoạt động',
    technical: 'Kỹ thuật',
    faq: 'FAQ',
    github: 'GitHub',
  },
  hero: {
    kicker: 'Trình phát nhạc tự động cho Sky · Windows 10/11',
    titleLines: ['Chơi bản nhạc.', 'Không phải bàn phím.'],
    description: 'Nạp một sheet nhạc Sky, chuyển sang game và để từng nốt, hợp âm cùng nốt ngân được phát đúng thời điểm.',
    primaryCta: 'Tải xuống cho Windows',
    secondaryCta: 'Xem cách hoạt động',
    metadata: ['JSON', 'SKYSHEET', 'TXT', 'MÃ NGUỒN MỞ', 'PORTABLE', 'KHÔNG CẦN CÀI ĐẶT'],
    riskNote: 'Tự động phát nhạc có thể xung đột với Điều khoản Dịch vụ của Sky.',
    riskNoteLink: 'Hãy dùng có trách nhiệm và tự chịu rủi ro.',
  },
  playback: {
    kicker: 'Được xây dựng quanh bản nhạc',
    title: 'Timing cũng là một phần của nhạc cụ.',
    description: 'Sheet nhạc không chỉ là một danh sách phím. Hợp âm phải vang cùng lúc, đoạn nhanh cần khoảng cách ổn định và nốt ngân phải giữ đủ trường độ. Sky Auto Player lên lịch các sự kiện âm nhạc như một màn trình diễn thay vì phát lại một macro chung chung.',
    points: ['Hợp âm căn theo frame', 'Phát theo tempo', 'Nốt, hợp âm và nốt ngân', 'Xem thử bằng dry-run'],
  },
  comparison: {
    kicker: 'Không phải macro chung chung',
    title: 'Xây cho âm nhạc, không phải chuỗi click.',
    macroHeader: 'Macro thông thường',
    playerHeader: 'Sky Auto Player',
    rows: [
      { macro: 'Gửi phím tuần tự', player: 'Hợp âm được căn vào cùng một frame gửi' },
      { macro: 'Độ trễ cố định', player: 'Timing đi theo sheet và tempo' },
      { macro: 'Chủ yếu giả định bấm-thả', player: 'Hỗ trợ nốt, hợp âm và nốt ngân' },
      { macro: 'Một cấu hình dùng cho mọi bài', player: 'Profile timing riêng cho từng bài' },
    ],
  },
  product: {
    kicker: 'Ứng dụng thực tế',
    title: 'Thư viện, profile timing và điều khiển trong cùng một giao diện.',
    description: 'Trình chọn điều khiển bằng bàn phím giữ tìm kiếm bài hát, thiết lập phát và trạng thái trong một nơi mà không cần giao diện desktop nặng nề.',
    annotations: [
      'Tìm và chọn bài trong trình chọn terminal.',
      'Xem profile timing được gợi ý trước khi phát.',
      'Luôn có sẵn điều khiển tạm dừng, bỏ qua và dừng.',
    ],
  },
  steps: {
    kicker: 'Ba bước',
    title: 'Từ tải xuống đến phát nhạc chỉ trong vài phút.',
    items: [
      {
        title: 'Tải xuống',
        description: 'Tải file ZIP mới nhất từ GitHub Releases và giải nén vào thư mục bạn chọn. Không cần trình cài đặt hệ thống hoặc quyền quản trị.',
      },
      {
        title: 'Thêm sheet nhạc',
        description: 'Xuất sheet dạng JSON, .skysheet hoặc TXT tương thích từ Sky Music editor rồi đặt vào thư mục songs.',
      },
      {
        title: 'Phát nhạc',
        description: 'Mở Sky Auto Player, chọn bài hát rồi chuyển sang cửa sổ Sky khi bạn đã sẵn sàng.',
      },
    ],
    hotkeyNote: 'Ctrl+R tải lại thư viện · F8 tạm dừng · F9 bỏ qua · F10 dừng',
  },
  technical: {
    kicker: 'Giới hạn kỹ thuật',
    title: 'Rõ ràng về những gì ứng dụng làm và không làm.',
    description: 'Sky Auto Player chạy như một ứng dụng Windows độc lập và gửi các sự kiện đầu vào tiêu chuẩn. Mã nguồn được công khai để mọi người có thể trực tiếp kiểm tra cách triển khai.',
    ledger: [
      { term: 'Đầu vào', definition: 'Windows SendInput' },
      { term: 'Tiến trình', definition: 'Ứng dụng độc lập' },
      { term: 'Bộ nhớ game', definition: 'Không được đọc/kiểm tra' },
      { term: 'Inject code', definition: 'Không sử dụng' },
      { term: 'File game', definition: 'Không sửa đổi' },
      { term: 'Giấy phép', definition: 'GNU GPL v3.0' },
      { term: 'Cập nhật', definition: 'Trình cập nhật riêng có xác minh checksum' },
    ],
    notice: 'Điều khoản Dịch vụ: Các giới hạn kỹ thuật trên không bảo đảm an toàn cho tài khoản. Tự động phát nhạc vẫn có thể xung đột với Điều khoản Dịch vụ của Sky. Hãy dùng công cụ có trách nhiệm và tự chịu rủi ro.',
  },
  formats: {
    kicker: 'Sheet được hỗ trợ',
    title: 'Nạp các định dạng cộng đồng đang sử dụng.',
    items: [
      {
        extension: '.json',
        name: 'JSON',
        description: 'Sheet JSON có cấu trúc, chứa sự kiện âm nhạc và metadata mà trình phát hỗ trợ.',
        tags: 'NỐT · HỢP ÂM · NGÂN',
      },
      {
        extension: '.skysheet',
        name: 'Skysheet',
        description: 'Sheet dựa trên JSON với phần mở rộng .skysheet từ hệ sinh thái trình soạn nhạc Sky.',
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
    kicker: 'Trước khi tải xuống',
    title: 'Một vài câu trả lời hữu ích trước.',
    readMoreLink: 'Đọc FAQ đầy đủ',
  },
  finalCta: {
    title: 'Màn trình diễn tiếp theo đã nằm sẵn trong sheet.',
    description: 'Tải Sky Auto Player, thêm một sheet và để ứng dụng xử lý phần timing.',
    primaryCta: 'Tải bản mới nhất',
    secondaryCta: 'Xem mã nguồn trên GitHub',
  },
};

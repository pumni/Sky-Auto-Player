/** Public donation destinations, intentionally kept separate from persisted settings. */
export const DONATION_CONFIG = {
  koFiUrl: 'https://ko-fi.com/pumni',
  paypalUrl: 'https://www.paypal.com/paypalme/pe4ce28',
  githubUrl: 'https://github.com/pumni/Sky-Auto-Player',
  bankName: 'Techcombank',
  bankBin: '970407',
  accountNumber: '6708022002',
  accountHolder: 'Nguyễn Đức Hòa',
  transferMemo: 'Sky Auto Player Donate',
  // Bundled EMVCo/NAPAS VietQR: static amount, purpose prefilled (tag 62/08).
  vietQrImagePath: '/donate-vietqr-techcombank.png',
} as const;

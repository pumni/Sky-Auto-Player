/** Voluntary donations only; publicly displayed, never persisted in native settings. */
export const DONATION_CONFIG = {
  koFiUrl: 'https://ko-fi.com/pumni',
  paypalUrl: 'https://www.paypal.com/paypalme/pe4ce28',
  githubUrl: 'https://github.com/pumni/Sky-Auto-Player',
  bankName: 'MBBank',
  bankBin: '970422',
  accountNumber: '0857288363',
  accountHolder: 'Nguyen Duc Hoa',
  transferMemo: 'Sky Auto Player Donate',
  // Offline NAPAS QRIBFTTA transfer to this bank account. No fixed amount.
  vietQrImagePath: '/donate-vietqr-mbbank.png',
} as const;

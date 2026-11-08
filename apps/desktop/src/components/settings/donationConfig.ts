/** Public, voluntary donation destinations. No payment processing or persisted settings. */
export const DONATION_CONFIG = {
  koFiUrl: 'https://ko-fi.com/pumni',
  paypalUrl: 'https://www.paypal.com/paypalme/pe4ce28',
  githubUrl: 'https://github.com/pumni/Sky-Auto-Player',
  momoPhoneNumber: '0857288363',
  momoAccountHolder: 'Nguyễn Đức Hòa',
  // The PSP receiving account is embedded in the owner-provided MoMo/NAPAS QR.
  momoReceivingId: 'PSP2603710800000415',
  transferMemo: 'Sky Auto Player Donate',
  // Cropped from the MoMo receive QR provided by the owner, without changing its payload.
  momoQrImagePath: '/donate-momo-receive.png',
} as const;

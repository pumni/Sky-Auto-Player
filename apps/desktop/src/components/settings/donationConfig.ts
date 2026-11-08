/** Public, voluntary donation destinations. No payment processing or persisted settings. */
export const DONATION_CONFIG = {
  koFiUrl: 'https://ko-fi.com/pumni',
  paypalUrl: 'https://www.paypal.com/paypalme/pe4ce28',
  githubUrl: 'https://github.com/pumni/Sky-Auto-Player',
  momoPhoneNumber: '0857288363',
  momoAccountHolder: 'Nguyễn Đức Hòa',
  // Account identifier copied from MoMo; not a substitute for the official receive QR.
  momoReceivingId: 'PSP2603710800000415',
  transferMemo: 'Sky Auto Player Donate',
  // Set only after importing the actual NEW receive QR image exported from the user's MoMo app.
  momoQrImagePath: null as string | null,
} as const;

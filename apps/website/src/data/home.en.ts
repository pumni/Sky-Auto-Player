import type { HomeContent } from './home.types';

export const homeEn: HomeContent = {
  locale: 'en',
  downloadCta: 'Download for Windows',
  seo: {
    title: 'Sky Auto Player — Music Sheet Player for Sky on Windows 11',
    description:
      'A free, open-source music sheet player for Sky on Windows 11. Bring your own sheets or choose from the built-in song library.',
  },
  navigation: {
    overview: 'Overview',
    howItWorks: 'How it works',
    guides: 'Guides',
    faq: 'FAQ',
    github: 'GitHub',
    release: 'Download',
  },
  hero: {
    eyebrow: 'A music player for Sky',
    titleStart: 'Bring ',
    titleEmphasis: 'your music',
    titleEnd: ' into Sky.',
    description:
      'Automatically play music sheets in Sky: Children of the Light. Choose a song or import your own sheet.',
    secondaryCta: 'Explore the player',
    meta: 'Windows 11 x64 · Free and open source · GitHub Releases',
    riskNote: 'Unofficial project. Automated playback may conflict with Sky’s Terms of Service.',
  },
  product: {
    title: 'Explore the player',
    description: 'Browse and prepare your sheets.',
    selectorLabel: 'Choose a player view',
    fullSizeLabel: 'View full size',
    fallbackLabel: 'Open another player view',
    pendingMessage: 'Loading preview…',
    errorMessage: 'The preview could not load. Try again or open it directly.',
    captureNote:
      'Desktop interface preview · Sample data · Rendered in browser, not a native capture',
    features: [
      {
        title: 'Bring your own sheets',
        description:
          'Import compatible files or folders into playlists. Your original files stay where they are.',
      },
      {
        title: 'Check compatibility before playing',
        description:
          'Inspect sheet details, choose supported tracks or pitch where available, and review possible conversion losses.',
      },
    ],
    views: [
      {
        label: 'Library',
        alt: 'Browser mock preview of the desktop song library with example songs and a playlist sidebar.',
        caption: 'Search bundled songs, save favorites and organize playlists.',
        src: '/assets/images/desktop-library-browser-preview-1366x768.png',
      },
      {
        label: 'Song details',
        alt: 'Browser mock preview of a selected example song and its sheet details in the desktop interface.',
        caption:
          'Review sheet compatibility before playback. Blue Bird is selected in this preview.',
        src: '/assets/images/desktop-song-detail-browser-preview-1366x768.png',
      },
      {
        label: 'Playback settings',
        alt: 'Browser mock preview of the desktop playback settings with example data.',
        caption: 'Adjust playback defaults and key-hold duration.',
        src: '/assets/images/desktop-settings-playback-browser-preview-1366x768.png',
      },
    ],
  },
  playback: {
    title: 'Shape your playback',
    description: 'Play notes, chords and holds from the sheet. Adjust tempo and key-hold duration.',
    dryRunTitle: 'Try your settings first',
    dryRunNote:
      'In the desktop app, Test playback (no input) runs a preview without sending key presses.',
    eventTypes: { chord: 'Chord', note: 'Note', hold: 'Hold' },
    controlsLabel: 'Playback demonstration controls',
    play: 'Run demo',
    resume: 'Continue phrase',
    pause: 'Pause phrase',
    replay: 'Replay phrase',
    previous: 'Previous event',
    next: 'Next event',
    progressLabel: 'Phrase timeline progress',
    keyboardLabel: 'Keys',
    initialStatus: 'Ready. The phrase contains a chord, a note, and a held note.',
    playingStatus: 'Playing',
    pausedStatus: 'Paused',
    completeStatus: 'Phrase complete. No keys are active.',
    manualOnly: 'Reduced motion is on. Use Previous and Next to step through the phrase.',
    demoNote: '2.6-second visual demo. No audio or in-game timing measurement.',
    timingLinkLabel: 'How timing works',
    secondsAbbreviation: 's',
  },
  quickStart: {
    title: 'Start playing in three steps',
    items: [
      {
        title: 'Install',
        description: 'Get the stable Windows installer from official GitHub Releases.',
      },
      {
        title: 'Choose a song',
        description: 'Choose a built-in song or import a compatible sheet.',
      },
      {
        title: 'Play in Sky',
        description:
          'Open a compatible 15-key instrument in Sky. Choose a song, press Play in the app and return focus to the Sky window.',
      },
    ],
    setupGuideLabel: 'Read the setup guide',
    setupGuideHref: '/guides/windows-setup/',
    formatsLabel: 'Formats',
    formatsValue: '.json · .skysheet · JSON-compatible .txt',
    formatsLinkLabel: 'Supported formats',
    formatsHref: '/guides/sheet-formats/',
  },
  trust: {
    title: 'Get Sky Auto Player',
    platformSummary: 'Windows 11 x64 · Free and open source',
    releaseDestinationNote: 'Opens GitHub Releases',
    installationTitle: 'Installation',
    installerGuidance: 'Choose the stable Windows installer ending in',
    installerSuffix: '_x64-setup.exe',
    signatureNote: 'The .exe.sig file is for the updater, not manual installation.',
    installScope: 'Installs for the current Windows user.',
    signingValue: 'Unsigned installer; Windows may show Unknown Publisher or SmartScreen.',
    safetyNote:
      'Uses Windows SendInput. It does not inspect game memory or change game files; this cannot guarantee account safety.',
    safetyTitle: 'Account safety',
    safetyLink: 'Read about account risk',
    supportLabel: 'Help',
    supportLinks: [
      { label: 'Setup', href: '/guides/windows-setup/' },
      { label: 'FAQ', href: '/faq/' },
      { label: 'Troubleshooting', href: '/guides/troubleshooting/' },
    ],
  },
};

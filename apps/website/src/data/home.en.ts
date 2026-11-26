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
    overview: 'Product',
    howItWorks: 'Getting started',
    guides: 'Guides',
    faq: 'FAQ',
    github: 'GitHub',
    release: 'Download',
  },
  hero: {
    eyebrow: 'Free and open source',
    titleStart: 'Your music.',
    titleEnd: 'Into Sky.',
    description:
      'Automatically play music in Sky: Children of the Light. Choose a built-in song or import your own sheet.',
    secondaryCta: 'Getting started',
    meta: 'Windows 11 x64 · GitHub Releases',
    riskNote: 'Unofficial project. Automated playback may conflict with Sky’s Terms of Service.',
  },
  product: {
    title: 'Explore the player',
    selectorLabel: 'Choose a player view',
    fullSizeLabel: 'Full size',
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
    kicker: 'Playback & timing',
    demoTitle: 'Playback demo',
    title: 'Make the timing yours.',
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
    eyebrow: 'Getting started',
    title: 'From sheet to song.',
    description: 'Set up the player, choose your music and play it in Sky in three steps.',
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
    formatsLabel: 'Supported sheet formats',
    formats: ['.json', '.skysheet', '.txt'],
    formatsNote: 'TXT files must contain compatible JSON.',
    formatsLinkLabel: 'Read the format guide',
    formatsHref: '/guides/sheet-formats/',
  },
  trust: {
    title: 'Ready to play?',
    platformSummary: 'Windows 11 x64 · Free and open source',
    releaseLinkLabel: 'View stable release',
    releaseDestinationNote: 'Opens GitHub Releases',
    installationTitle: 'Choose the right installer',
    installerGuidance: 'Choose the stable Windows installer ending in',
    installerSuffix: '_x64-setup.exe',
    signatureNote: 'The .exe.sig file is for the updater, not manual installation.',
    installScope: 'Installs for the current Windows user.',
    signingValue: 'Unsigned installer; Windows may show Unknown Publisher or SmartScreen.',
    safetyNote: 'Sends keystrokes using Windows SendInput.',
    safetyBoundary: 'Does not inspect game memory or change game files.',
    safetyRisk: 'This cannot guarantee account safety.',
    safetyTitle: 'Account safety',
    safetyLink: 'Read about account risk',
    supportLabel: 'Need a hand?',
    supportDescription: 'Find answers before your first song or get back to playing.',
    supportLinks: [
      {
        label: 'Common questions',
        description: 'Setup, formats, compatibility and account safety.',
        href: '/faq/',
      },
      {
        label: 'Troubleshoot playback',
        description: 'Resolve playback, timing and input issues.',
        href: '/guides/troubleshooting/',
      },
    ],
  },
};

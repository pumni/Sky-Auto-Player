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
    titleStart: 'Bring ',
    titleEmphasis: 'your music',
    titleEnd: ' into Sky.',
    description:
      'Play music sheets in Sky: Children of the Light. Explore the built-in song library or import your own sheets, then prepare playback in one desktop app.',
    secondaryCta: 'Explore the player',
    meta: 'Windows 11 x64 · Free and open source · GitHub Releases',
    riskNote: 'Unofficial project. Automated playback may conflict with Sky’s Terms of Service.',
  },
  product: {
    title: 'Your music, your way.',
    description: 'Find a song, organize your collection and prepare playback.',
    selectorLabel: 'Choose a player view',
    fullSizeLabel: 'View full size',
    fallbackLabel: 'Open another player view',
    pendingMessage: 'Loading preview…',
    errorMessage: 'The preview could not load. Try again or open it directly.',
    captureNote: 'Desktop interface preview with sample data (browser mock, not a native capture).',
    views: [
      {
        label: 'Library',
        description: 'Browse built-in songs, favorites and playlists.',
        alt: 'Browser mock preview of the desktop song library with example songs and a playlist sidebar.',
        caption: 'Built-in songs in the sample library.',
        src: '/assets/images/desktop-library-browser-preview-1366x768.png',
      },
      {
        label: 'Song details',
        description: 'Review your sheet before playback.',
        alt: 'Browser mock preview of a selected example song and its sheet details in the desktop interface.',
        caption: 'Blue Bird selected with Inspector Details open.',
        src: '/assets/images/desktop-song-detail-browser-preview-1366x768.png',
      },
      {
        label: 'Playback settings',
        description: 'Adjust playback preferences for your setup.',
        alt: 'Browser mock preview of the desktop playback settings with example data.',
        caption: 'Playback defaults and timing options in Settings.',
        src: '/assets/images/desktop-settings-playback-browser-preview-1366x768.png',
      },
    ],
  },
  playback: {
    title: 'More than a sequence of keys.',
    description: 'See how a sheet schedules a chord, a single note and a held note over time.',
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
    schematicNote: 'Visual illustration · 2.6 seconds · No audio or in-game timing measurement.',
    timingLinkLabel: 'How timing works',
    secondsAbbreviation: 's',
  },
  quickStart: {
    title: 'From sheet to playback.',
    items: [
      {
        title: 'Install Sky Auto Player',
        description: 'Get the stable Windows installer from official GitHub Releases.',
      },
      {
        title: 'Choose your music',
        description: 'Browse the library or import a compatible sheet.',
      },
      {
        title: 'Get ready in Sky',
        description:
          'Prepare a supported instrument, select the Sky window and follow the playback guide.',
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
    title: 'Ready to bring your music into Sky?',
    description: 'Get the stable Windows release and start with your own music collection.',
    installerGuidance:
      'On GitHub Releases, choose the stable Windows x64 installer ending in _x64-setup.exe. The .exe.sig file is for the updater, not manual installation.',
    packageValue: 'Windows 11 x64 · Free and open source · current-user install',
    signingValue: 'Unsigned installer; Windows may show Unknown Publisher or SmartScreen.',
    safetyNote:
      'Uses Windows SendInput. It does not inspect game memory or change game files; this cannot guarantee account safety.',
    safetyLink: 'Account safety',
    supportLabel: 'Help',
    supportLinks: [
      { label: 'Setup', href: '/guides/windows-setup/' },
      { label: 'FAQ', href: '/faq/' },
      { label: 'Troubleshooting', href: '/guides/troubleshooting/' },
    ],
  },
};

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
    release: 'Releases',
  },
  hero: {
    title: 'Play the sheet. Not the keyboard.',
    description:
      'Play music sheets in Sky with a built-in song library and support for your own sheets.',
    secondaryCta: 'See the player',
    meta: 'Windows 11 x64 · Free and open source · GitHub Releases',
    riskNote: 'Unofficial project. Automated playback may conflict with Sky’s Terms of Service.',
  },
  product: {
    sectionLabel: 'Sky Auto Player desktop interface',
    selectorLabel: 'Choose a player view',
    fullSizeLabel: 'View full size',
    fallbackLabel: 'Open another player view',
    views: [
      {
        label: 'Library',
        alt: 'Desktop interface capture showing the Sky Auto Player song library and playlists.',
        caption: 'Browse songs, favorites, and playlists.',
        src: '/assets/images/library-real-tauri.png',
      },
      {
        label: 'Song Detail',
        alt: 'Desktop interface capture showing a selected song and its sheet details.',
        caption: 'Review a song before you play it.',
        src: '/assets/images/detail-real-tauri.png',
      },
      {
        label: 'Settings',
        alt: 'Desktop interface capture showing playback and application settings.',
        caption: 'Choose playback options for your setup.',
        src: '/assets/images/settings-real-tauri.png',
      },
    ],
  },
  playback: {
    title: 'Timing is the instrument.',
    description: 'See how a chord, single note and held note follow a sheet.',
    eventTypes: { chord: 'Chord', note: 'Note', hold: 'Hold' },
    controlsLabel: 'Playback demonstration controls',
    play: 'Play phrase',
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
    schematicNote: 'Illustration only · 2.6 seconds · Not a game timing measurement.',
    timingLinkLabel: 'How timing works',
    secondsAbbreviation: 's',
  },
  quickStart: {
    title: 'Start playing in three steps.',
    items: [
      {
        title: 'Install',
        description: 'Get the Windows installer from GitHub Releases.',
      },
      {
        title: 'Choose music',
        description: 'Pick a song or import your own sheet.',
      },
      {
        title: 'Play',
        description: 'Select the Sky window and use Play as instructed.',
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
    title: 'Ready to play?',
    packageLabel: 'Package',
    packageValue: 'Windows 11 x64 · NSIS · current-user install',
    signingValue: 'Unsigned installer; Windows may show Unknown Publisher or SmartScreen.',
    updaterValue: 'Tauri updates separately verify the .exe.sig sidecar.',
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

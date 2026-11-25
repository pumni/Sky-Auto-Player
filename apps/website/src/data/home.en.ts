import type { HomeContent } from './home.types';

export const homeEn: HomeContent = {
  locale: 'en',
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
    kicker: 'A free, open-source music sheet player for Sky on Windows 11',
    titleLines: ['Play the sheet.', 'Not the keyboard.'],
    description:
      'Choose a song from the built-in library or bring a sheet of your own. Set up a playlist, make the music yours, and let Sky Auto Player handle the notes.',
    primaryCta: 'Get the Windows installer',
    primaryCtaDestination: 'Opens GitHub Releases',
    secondaryCta: 'See the player',
    affiliationDisclaimer:
      'Unofficial community project. Not affiliated with or endorsed by thatgamecompany.',
  },
  benefits: {
    kicker: 'Made for the way you play',
    items: [
      {
        number: '01',
        title: 'Play from the sheet',
        description: 'Notes, chords, and held notes follow the music written in each sheet.',
      },
      {
        number: '02',
        title: 'Keep your songs together',
        description: 'Browse the built-in song library, like favorites, and organize playlists.',
      },
      {
        number: '03',
        title: 'Bring your own sheets',
        description: 'Import supported files into a playlist while their original files stay put.',
      },
      {
        number: '04',
        title: 'Open source, free to use',
        description: 'Inspect the project on GitHub and download the Windows app at no cost.',
      },
    ],
  },
  product: {
    kicker: 'Inside the player',
    title: 'A place for your songs, sheets, and settings.',
    description:
      'Move between the library, a song’s details, and playback settings. Select a desktop interface capture to take a closer look.',
    selectorLabel: 'Choose a player view',
    openImageLabel: 'Open this desktop interface capture at full size',
    fallbackLabel: 'Open a desktop interface capture',
    captureLabel: 'Desktop interface capture',
    views: [
      {
        id: 'library',
        label: 'Library',
        alt: 'Desktop interface capture showing the Sky Auto Player song library and playlists.',
        caption: 'Browse songs, liked tracks, and playlists.',
        src: '/assets/images/library-real-tauri.png',
      },
      {
        id: 'song-detail',
        label: 'Song Detail',
        alt: 'Desktop interface capture showing a selected song and its sheet details.',
        caption: 'Review a song before you play it.',
        src: '/assets/images/detail-real-tauri.png',
      },
      {
        id: 'settings',
        label: 'Settings',
        alt: 'Desktop interface capture showing playback and application settings.',
        caption: 'Choose playback options for your setup.',
        src: '/assets/images/settings-real-tauri.png',
      },
    ],
  },
  quickStart: {
    kicker: 'From download to first song',
    title: 'Three steps to make the music yours.',
    items: [
      {
        title: 'Install the player',
        description:
          'Get the NSIS installer from the official GitHub Releases page. It installs for the current Windows user.',
        linkLabel: 'Windows setup guide',
        href: '/guides/windows-setup/',
      },
      {
        title: 'Choose a song or add a sheet',
        description:
          'Pick a song from the built-in library, or add a JSON, .skysheet, or JSON-compatible TXT file to a playlist.',
        linkLabel: 'Sheet format guide',
        href: '/guides/sheet-formats/',
      },
      {
        title: 'Play with Sky in focus',
        description:
          'Select the Sky window, choose a song, and press Play. The app checks the foreground and retained Sky target before each new musical input packet.',
        linkLabel: 'How playback works',
        href: '/guides/how-it-works/',
      },
    ],
  },
  playback: {
    kicker: 'A musical moment, step by step',
    title: 'Watch a phrase move from note to note.',
    description:
      'Play this short schematic phrase, step through each event yourself, or replay it. The event list, highlighted keys, elapsed time, and held note stay in sync.',
    eventListLabel: 'Playback events',
    eventTypes: { chord: 'Chord', note: 'Note', hold: 'Held note' },
    controlsLabel: 'Playback demonstration controls',
    play: 'Play demonstration',
    resume: 'Continue demonstration',
    pause: 'Pause demonstration',
    replay: 'Replay demonstration',
    cancel: 'Cancel playback',
    previous: 'Previous event',
    next: 'Next event',
    progressLabel: 'Demonstration progress',
    timelineLabel: 'Illustrative phrase timeline',
    keyboardLabel: 'ILLUSTRATIVE KEYBOARD',
    initialStatus: 'Ready. The first chord is shown; start playback or step through the phrase.',
    staticExplanation:
      'Static example: the phrase contains a chord, a note, and a held note, in that order.',
    playingStatus: 'Demonstration playing',
    pausedStatus: 'Demonstration paused',
    completeStatus: 'Demonstration complete. No keys are active.',
    manualOnly:
      'Motion is reduced on this device. Use the event controls to step through the phrase.',
    schematicNote:
      'Schematic 2.6-second example. These intervals illustrate the interface; they are not song timing or a measurement of in-game input.',
  },
  technical: {
    kicker: 'Two ways to describe a sequence',
    title: 'Fixed intervals and sheet events serve different workflows.',
    description:
      'A fixed-delay sequence repeats intervals chosen in advance. Sky Auto Player reads authored note, chord, and hold events from a sheet and schedules them against QPC deadlines with a calibrated fixed wake guard. It measures sender-side completion without continuously adjusting its lead.',
    fixedDelayTitle: 'Fixed-delay sequence',
    sheetTitle: 'Sheet-event playback',
    dimensionHeader: 'Dimension',
    dimensions: [
      {
        label: 'What you prepare',
        fixedDelay: 'A key sequence and its intervals',
        sheet: 'A song sheet with musical events',
      },
      {
        label: 'What repeats',
        fixedDelay: 'The chosen interval pattern',
        sheet: 'The notes, chords, and holds authored in the sheet',
      },
      {
        label: 'What you can change',
        fixedDelay: 'The sequence and its delays',
        sheet: 'The song, tempo, and Base Hold in the playback profile',
      },
    ],
    boundaryNote:
      'Windows scheduling and Sky’s own input sampling determine when a key registers in game. Sender-side measurements do not guarantee in-game sound or frame timing.',
  },
  formats: {
    kicker: 'Bring a sheet',
    title: 'Start with the formats the player supports.',
    items: [
      {
        extension: '.json',
        name: 'JSON',
        description: 'Structured song sheets with supported musical events and metadata.',
        tags: 'NOTE · CHORD · HOLD',
      },
      {
        extension: '.skysheet',
        name: 'Skysheet',
        description: 'A JSON-based song sheet using the .skysheet file extension.',
        tags: 'SHEET FILE',
      },
      {
        extension: '.txt',
        name: 'JSON-compatible TXT',
        description: 'A plain-text file containing a JSON-compatible sheet structure.',
        tags: 'PLAIN TEXT',
      },
    ],
  },
  trust: {
    kicker: 'Download with the details in view',
    title: 'Know what you are installing.',
    description:
      'Sky Auto Player is a separate Windows app. Check the release notes and package details on GitHub before installing.',
    downloadLabel: 'Get the Windows installer',
    downloadDestination: 'Opens GitHub Releases',
    releaseLabel: 'Published version',
    releaseValue: 'v',
    packageLabel: 'Package',
    packageValue: 'Windows 11 x64 · NSIS installer · current-user install',
    signingLabel: 'Authenticode',
    signingValue:
      'Installer is intentionally unsigned; Windows may show Unknown Publisher or SmartScreen.',
    updaterLabel: 'App updates',
    updaterValue: 'Tauri updater separately verifies the .exe.sig sidecar.',
    sourceLabel: 'Read the source on GitHub',
    safetyNote:
      'The app does not inspect game memory, change game files, inject code, install input hooks, or attach a debugger. These boundaries do not guarantee account safety; automated playback may conflict with Sky’s Terms of Service.',
    safetyLink: 'Read the safety details',
  },
  faqPreview: {
    kicker: 'A few things worth knowing',
    title: 'Questions before you begin.',
    readMoreLink: 'Read the full FAQ',
    items: [
      { question: 'Is Sky Auto Player free and open source?', href: '/faq/#free' },
      { question: 'Which sheet formats can I use?', href: '/faq/#formats' },
      { question: 'Could automated playback affect my account?', href: '/faq/#account-safety' },
    ],
    guideLinks: [
      { label: 'How playback works', href: '/guides/how-it-works/' },
      { label: 'Troubleshoot an issue', href: '/guides/troubleshooting/' },
    ],
  },
  finalCta: {
    title: 'Bring a song to Sky.',
    description: 'Explore the player, choose a sheet, and start with the official Windows release.',
    primaryCta: 'Get the Windows installer',
    primaryDestination: 'Opens GitHub Releases',
    secondaryCta: 'Explore the source',
  },
};

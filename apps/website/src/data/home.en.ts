import type { HomeContent } from './home.types';

export const homeEn: HomeContent = {
  locale: 'en',
  seo: {
    title: 'Sky Auto Player — Timing-First Music Player for Sky on Windows 11',
    description:
      'Sheet playback for Sky on Windows 11 x64, scheduled against QPC deadlines with sender-side timing measurements and Windows SendInput.',
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
    kicker: 'Sky Auto Player for Sky: Children of the Light · Windows 11 x64',
    titleLines: ['Play the sheet.', 'Not the keyboard.'],
    description:
      'Load a sheet and choose a song. Sky Auto Player schedules sheet events against QPC deadlines. Each new musical packet is checked against the current foreground window and retained Sky target before Windows SendInput; bounded cleanup for journaled key releases follows a separate path. Sky samples input on its own schedule.',
    primaryCta: 'Get the Windows installer',
    primaryCtaDestination: 'Opens GitHub Releases',
    secondaryCta: 'See how it works',
    metadata: ['JSON', 'SKYSHEET', 'TXT', 'FREE · OPEN SOURCE', 'WINDOWS 11 X64', 'TAURI DESKTOP'],
    riskNote: 'Automated playback may conflict with Sky’s Terms of Service.',
    riskNoteLink: 'Use responsibly and at your own risk.',
    affiliationDisclaimer:
      'Unofficial community project. Not affiliated with or endorsed by thatgamecompany.',
  },
  proofStrip: {
    kicker: 'Playback model',
    metric: 'SHEET EVENTS / SENDINPUT',
    signals: [
      'A healthy chord is sent in one prepared packet',
      'A calibrated fixed wake guard; no continuous lead adjustment',
      'Frame-aware hold and release floors',
      'HUD reports sender-side measurements, not game-side sampling',
    ],
  },
  playback: {
    kicker: 'Built around the music',
    title: 'Timing is the instrument.',
    description:
      "Sky Auto Player schedules sheet events against authored QPC deadlines and uses a calibrated fixed wake guard. It measures sender-side completion but does not continuously adjust its lead. Frame-aware hold and release floors shape musical key timing; Windows scheduling and Sky's input sampling still determine when the game registers a key, so the app cannot guarantee in-game sound or frame timing.",
    points: [
      'Authored sheet events scheduled against QPC deadlines',
      'A calibrated fixed wake guard with no continuous lead adjustment',
      'Frame-aware hold and release floors',
      'Tempo and Base Hold in the Player Dock playback profile',
      'Sender-side timing measurements in the HUD',
      'Dry-run preview',
    ],
  },
  comparison: {
    kicker: 'From sheet event to input',
    title: 'Playback follows the sheet.',
    sequenceHeader: 'Fixed-delay sequence',
    playerHeader: 'Sky Auto Player',
    rows: [
      { sequence: 'A sequence of key presses', player: 'Sheet-based note, chord, and hold events' },
      {
        sequence: 'One fixed interval pattern',
        player: 'Authored events scheduled against QPC deadlines',
      },
      { sequence: 'One tempo for every song', player: 'Tempo selected in the Player Dock profile' },
      { sequence: 'One hold duration', player: 'Base Hold selected in the Player Dock profile' },
      {
        sequence: 'No sender feedback',
        player: 'Fixed wake guard plus sender-side measurements',
      },
      {
        sequence: 'Timing set only by the sequence',
        player: 'Diagnostics report sender-side timing measurements',
      },
    ],
  },
  product: {
    kicker: 'The actual player',
    title: 'Your library, timing profile and controls in one place.',
    description:
      'The Windows desktop app includes a built-in song catalog, liked songs, playlists, and diagnostics.',
    annotations: [
      'Browse built-in songs and liked songs in the Library.',
      'Create or select a playlist, then use Add songs to import local files or a folder.',
      'Imports reference the original files; the app does not move or delete them.',
    ],
  },
  steps: {
    kicker: 'Three steps',
    title: 'Get started with a sheet.',
    items: [
      {
        title: 'Download',
        description:
          'Get the canonical Tauri NSIS installer from the official Sky Auto Player GitHub Releases page and install it for the current user.',
      },
      {
        title: 'Add a sheet',
        description:
          'Create or select a playlist, then use Add songs to import a JSON, .skysheet or JSON-compatible TXT file or folder. Imported sources stay in their original location.',
      },
      {
        title: 'Play',
        description:
          'Choose a song, use Play, and return focus to the selected Sky window. Fresh foreground and target identity checks gate each musical packet.',
      },
    ],
    workflowNote: 'Use / or Ctrl+F to focus search · Esc closes safe overlays',
  },
  technical: {
    kicker: 'Technical boundaries',
    title: 'Clear about what it does—and what it does not do.',
    description:
      'Sky Auto Player is a separate Windows 11 app. Each new musical packet requires fresh foreground and target identity checks before Windows SendInput. Bounded cleanup for journaled key releases follows a separate path; the public source is available for review.',
    ledger: [
      { term: 'Input', definition: 'Windows SendInput', state: 'yes' },
      { term: 'Process', definition: 'Separate application', state: 'info' },
      { term: 'Game memory', definition: 'Not inspected', state: 'no' },
      { term: 'Code injection', definition: 'Not used', state: 'no' },
      { term: 'Game files', definition: 'Not modified', state: 'no' },
      { term: 'License', definition: 'GNU GPL v3.0', state: 'info' },
      {
        term: 'Updates',
        definition: 'Tauri updater verifies the .exe.sig sidecar',
        state: 'info',
      },
    ],
    notice:
      'Terms of Service: These technical boundaries do not guarantee account safety. Automated music playback may still conflict with Sky’s Terms of Service. Use the tool responsibly and at your own risk.',
  },
  formats: {
    kicker: 'Supported sheets',
    title: 'Load the formats the community already uses.',
    items: [
      {
        extension: '.json',
        name: 'JSON',
        description:
          'Structured song sheets with musical events and metadata accepted by the player.',
        tags: 'NOTE · CHORD · HOLD',
      },
      {
        extension: '.skysheet',
        name: 'Skysheet',
        description:
          'JSON-based sheets with the .skysheet extension used by the Sky music editor ecosystem.',
        tags: 'SHEET EXPORT',
      },
      {
        extension: '.txt',
        name: 'JSON-compatible TXT',
        description: 'Plain-text files containing a JSON-compatible sheet structure.',
        tags: 'PLAIN TEXT',
      },
    ],
  },
  faqPreview: {
    kicker: 'Before you download',
    title: 'A few useful answers first.',
    readMoreLink: 'Read the full FAQ',
    items: [
      {
        question: 'Is Sky Auto Player free and open source?',
        href: '/faq/#free',
      },
      {
        question: 'Which sheet formats are supported?',
        href: '/faq/#formats',
      },
      {
        question: 'Can this affect my Sky account?',
        href: '/faq/#account-safety',
      },
    ],
    guideLinks: [
      { label: 'How Sky Auto Player works', href: '/guides/how-it-works/' },
      { label: 'Troubleshooting common issues', href: '/guides/troubleshooting/' },
    ],
  },
  finalCta: {
    title: 'Your next performance is already written.',
    description:
      'Browse the official release, add a sheet to a playlist and review the available playback settings.',
    primaryCta: 'Browse releases',
    secondaryCta: 'View source on GitHub',
  },
};

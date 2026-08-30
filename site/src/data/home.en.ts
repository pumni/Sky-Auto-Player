import type { HomeContent } from './home.types';

export const homeEn: HomeContent = {
  locale: 'en',
  seo: {
    title: 'Sky Auto Player — Timing-First Music Player for Sky on Windows',
    description:
      'Load a Sky music sheet and play notes, chords and holds on time — timing-first, sender-side latency compensated, no macro replay.',
  },
  navigation: {
    playback: 'Playback',
    howItWorks: 'How it works',
    technical: 'Technical',
    guides: 'Guides',
    faq: 'FAQ',
    github: 'GitHub',
  },
  hero: {
    kicker: 'Sky Auto Player for Sky: Children of the Light · Windows 10/11',
    titleLines: ['Play the sheet.', 'Not the keyboard.'],
    description:
      "Load a Sky music sheet, switch to the game, and let every note, chord, and hold land on the beat — scheduled from the sheet's timing model and dispatched through Windows SendInput.",
    primaryCta: 'Download for Windows',
    secondaryCta: 'See how it works',
    metadata: ['JSON', 'SKYSHEET', 'TXT', 'OPEN SOURCE', 'PORTABLE', 'NO INSTALLER'],
    riskNote: 'Automated playback may conflict with Sky’s Terms of Service.',
    riskNoteLink: 'Use responsibly and at your own risk.',
    affiliationDisclaimer:
      'Unofficial community project. Not affiliated with or endorsed by thatgamecompany.',
  },
  proofStrip: {
    kicker: 'Performance profile',
    metric: 'SENDINPUT BATCH / 60 FPS TARGET',
    signals: [
      'Chord notes submitted in one SendInput batch — sender-side skew minimised',
      'Adaptive lead learns sender-side completion delay on this machine',
      'Holds keep their full duration, never clipped',
      'Player and display on separate threads — HUD stays smooth',
    ],
  },
  playback: {
    kicker: 'Built around the music',
    title: 'Timing is the instrument.',
    description:
      'A sheet is more than a list of keys. Chords must arrive together, fast passages need consistent spacing, and holds need their full duration. Sky Auto Player schedules those musical events as a performance rather than replaying a generic macro. Dispatch timing is measured at sender-side completion — not just the scheduled timestamp.',
    points: [
      'Chord notes in one SendInput batch',
      'Tempo-aware playback',
      'Notes, chords and holds',
      'Sender-side latency learned per machine',
      'Player and HUD on separate threads',
      'Dry-run preview',
    ],
  },
  comparison: {
    kicker: 'Not a generic macro',
    title: 'Built for music, not click sequences.',
    macroHeader: 'Generic macro',
    playerHeader: 'Sky Auto Player',
    rows: [
      { macro: 'Sequential key presses', player: 'Chords aligned to one dispatch frame' },
      { macro: 'Fixed delays', player: 'Timing follows the sheet and tempo' },
      { macro: 'Tap-focused playback', player: 'Notes, chords and holds' },
      { macro: 'One global setup', player: 'Per-song timing profiles' },
      {
        macro: 'Fixed delay, one-size-fits-all',
        player: 'Learns your machine delay and compensates',
      },
      {
        macro: 'HUD stutters when busy',
        player: 'Note firing and display run on separate threads',
      },
    ],
  },
  product: {
    kicker: 'The actual player',
    title: 'Your library, timing profile and controls in one place.',
    description:
      'The Tauri desktop app keeps song search, playback setup and status visible in one focused window.',
    annotations: [
      'Search and select songs from the desktop Library.',
      'Review timing risk and recommendations in Song Detail.',
      'Keep playback, diagnostics and settings within reach.',
    ],
  },
  steps: {
    kicker: 'Three steps',
    title: 'From download to playback in minutes.',
    items: [
      {
        title: 'Download',
        description:
          'Get the latest ZIP from GitHub Releases and extract it to a folder you control. No system installer or administrator access is required.',
      },
      {
        title: 'Add a sheet',
        description:
          'Export a JSON, .skysheet or compatible TXT sheet from the Sky Music editor and place it in the songs folder.',
      },
      {
        title: 'Play',
        description:
          'Open Sky Auto Player, choose a song, then switch to the Sky window when you are ready.',
      },
    ],
    hotkeyNote:
      'Press Reload songs in the GUI · / or Ctrl+F focuses search · Esc closes safe overlays',
  },
  technical: {
    kicker: 'Technical boundaries',
    title: 'Clear about what it does—and what it does not do.',
    description:
      'Sky Auto Player runs as a separate Windows application and sends standard input events. Its source is public, so the implementation can be reviewed directly.',
    ledger: [
      { term: 'Input', definition: 'Windows SendInput', state: 'yes' },
      { term: 'Process', definition: 'Separate application', state: 'info' },
      { term: 'Game memory', definition: 'Not inspected', state: 'no' },
      { term: 'Code injection', definition: 'Not used', state: 'no' },
      { term: 'Game files', definition: 'Not modified', state: 'no' },
      { term: 'License', definition: 'GNU GPL v3.0', state: 'info' },
      {
        term: 'Updates',
        definition: 'Explicit updater with checksum verification',
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
    description: 'Download Sky Auto Player, add a sheet and let the timing take care of itself.',
    primaryCta: 'Download latest release',
    secondaryCta: 'View source on GitHub',
  },
};

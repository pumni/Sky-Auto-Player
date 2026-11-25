import type { Locale } from './site';

export interface HomeContent {
  locale: Locale;
  seo: { title: string; description: string };
  navigation: {
    overview: string;
    howItWorks: string;
    guides: string;
    faq: string;
    github: string;
    release: string;
  };
  hero: {
    kicker: string;
    titleLines: [string, string];
    description: string;
    primaryCta: string;
    primaryCtaDestination: string;
    secondaryCta: string;
    affiliationDisclaimer: string;
  };
  benefits: {
    kicker: string;
    items: { number: string; title: string; description: string }[];
  };
  product: {
    kicker: string;
    title: string;
    description: string;
    selectorLabel: string;
    openImageLabel: string;
    fallbackLabel: string;
    captureLabel: string;
    views: {
      id: 'library' | 'song-detail' | 'settings';
      label: string;
      alt: string;
      caption: string;
      src: string;
    }[];
  };
  quickStart: {
    kicker: string;
    title: string;
    items: { title: string; description: string; linkLabel: string; href: string }[];
  };
  playback: {
    kicker: string;
    title: string;
    description: string;
    eventListLabel: string;
    eventTypes: { chord: string; note: string; hold: string };
    controlsLabel: string;
    play: string;
    resume: string;
    pause: string;
    replay: string;
    cancel: string;
    previous: string;
    next: string;
    progressLabel: string;
    timelineLabel: string;
    keyboardLabel: string;
    initialStatus: string;
    staticExplanation: string;
    playingStatus: string;
    pausedStatus: string;
    completeStatus: string;
    manualOnly: string;
    schematicNote: string;
  };
  technical: {
    kicker: string;
    title: string;
    description: string;
    fixedDelayTitle: string;
    sheetTitle: string;
    dimensionHeader: string;
    dimensions: { label: string; fixedDelay: string; sheet: string }[];
    boundaryNote: string;
  };
  formats: {
    kicker: string;
    title: string;
    items: { extension: string; name: string; description: string; tags: string }[];
  };
  trust: {
    kicker: string;
    title: string;
    description: string;
    downloadLabel: string;
    downloadDestination: string;
    releaseLabel: string;
    releaseValue: string;
    packageLabel: string;
    packageValue: string;
    signingLabel: string;
    signingValue: string;
    updaterLabel: string;
    updaterValue: string;
    sourceLabel: string;
    safetyNote: string;
    safetyLink: string;
  };
  faqPreview: {
    kicker: string;
    title: string;
    readMoreLink: string;
    items: ReadonlyArray<{ question: string; href: string }>;
    guideLinks: ReadonlyArray<{ label: string; href: string }>;
  };
  finalCta: {
    title: string;
    description: string;
    primaryCta: string;
    primaryDestination: string;
    secondaryCta: string;
  };
}

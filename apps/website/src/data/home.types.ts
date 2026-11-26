import type { Locale } from './site';

export interface HomeContent {
  locale: Locale;
  downloadCta: string;
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
    title: string;
    description: string;
    secondaryCta: string;
    meta: string;
    riskNote: string;
  };
  product: {
    sectionLabel: string;
    selectorLabel: string;
    fullSizeLabel: string;
    fallbackLabel: string;
    captureNote: string;
    views: {
      label: string;
      alt: string;
      caption: string;
      src: string;
    }[];
  };
  quickStart: {
    title: string;
    items: { title: string; description: string }[];
    setupGuideLabel: string;
    setupGuideHref: string;
    formatsLabel: string;
    formatsValue: string;
    formatsLinkLabel: string;
    formatsHref: string;
  };
  playback: {
    title: string;
    description: string;
    eventTypes: { chord: string; note: string; hold: string };
    controlsLabel: string;
    play: string;
    resume: string;
    pause: string;
    replay: string;
    previous: string;
    next: string;
    progressLabel: string;
    keyboardLabel: string;
    initialStatus: string;
    playingStatus: string;
    pausedStatus: string;
    completeStatus: string;
    manualOnly: string;
    schematicNote: string;
    timingLinkLabel: string;
    secondsAbbreviation: string;
  };
  trust: {
    title: string;
    packageLabel: string;
    packageValue: string;
    signingValue: string;
    updaterValue: string;
    safetyNote: string;
    safetyLink: string;
    supportLabel: string;
    supportLinks: ReadonlyArray<{ label: string; href: string }>;
  };
}

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
    title: string;
    description: string;
    selectorLabel: string;
    fullSizeLabel: string;
    fallbackLabel: string;
    pendingMessage: string;
    errorMessage: string;
    captureNote: string;
    features: readonly [
      { title: string; description: string },
      { title: string; description: string },
    ];
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
    setupGuideTitle: string;
    setupGuideDescription: string;
    setupGuideLabel: string;
    setupGuideHref: string;
    formatsLabel: string;
    formats: readonly string[];
    formatsNote: string;
    formatsLinkLabel: string;
    formatsHref: string;
  };
  playback: {
    title: string;
    description: string;
    dryRunTitle: string;
    dryRunNote: string;
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
    demoNote: string;
    timingLinkLabel: string;
    secondsAbbreviation: string;
  };
  trust: {
    title: string;
    platformSummary: string;
    releaseLinkLabel: string;
    releaseDestinationNote: string;
    installationTitle: string;
    installerGuidance: string;
    installerSuffix: string;
    signatureNote: string;
    installScope: string;
    signingValue: string;
    safetyNote: string;
    safetyTitle: string;
    safetyLink: string;
    supportLabel: string;
    supportLinks: ReadonlyArray<{ label: string; href: string }>;
  };
}

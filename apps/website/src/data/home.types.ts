import type { Locale } from './site';

export interface HomeContent {
  locale: Locale;
  seo: {
    title: string;
    description: string;
  };
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
    metadata: string[];
    riskNote: string;
    riskNoteLink: string;
    /** Visible unofficial project disclaimer. */
    affiliationDisclaimer: string;
  };
  /** Dense transition strip under hero — product signals, not playback mechanism bullets. */
  proofStrip: {
    kicker: string;
    metric: string;
    signals: string[];
  };
  playback: {
    kicker: string;
    title: string;
    description: string;
    points: string[];
  };
  comparison: {
    kicker: string;
    title: string;
    sequenceHeader: string;
    playerHeader: string;
    rows: { sequence: string; player: string }[];
  };
  product: {
    kicker: string;
    title: string;
    description: string;
    annotations: string[];
  };
  steps: {
    kicker: string;
    title: string;
    items: { title: string; description: string }[];
    workflowNote: string;
  };
  technical: {
    kicker: string;
    title: string;
    description: string;
    ledger: { term: string; definition: string; state: 'yes' | 'no' | 'info' }[];
    notice: string;
  };
  formats: {
    kicker: string;
    title: string;
    items: { extension: string; name: string; description: string; tags: string }[];
  };
  faqPreview: {
    kicker: string;
    title: string;
    readMoreLink: string;
    items: ReadonlyArray<{
      question: string;
      /** Locale path without origin; may include hash. Example: "/faq/#download" */
      href: string;
    }>;
    /** Optional contextual guide cross-links below FAQ items. */
    guideLinks?: ReadonlyArray<{ label: string; href: string }>;
  };
  finalCta: {
    title: string;
    description: string;
    primaryCta: string;
    secondaryCta: string;
  };
}

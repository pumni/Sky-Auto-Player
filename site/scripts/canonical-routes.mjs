/**
 * canonical-routes.mjs
 *
 * Single authoritative source for all 18 canonical production routes and
 * 2 compatibility aliases. Imported by verify-dist.mjs and verify-seo.mjs.
 *
 * Declared explicitly — no file-existence conditional logic.
 * Adding a guide requires updating this list; removing one must remove from here.
 */

export const PRODUCTION_ORIGIN = 'https://pumni.github.io';
export const BASE = '/Sky-Auto-Player';

/** All 18 indexable canonical routes. */
export const CANONICAL_ROUTES = [
  // EN core
  { path: '/', lang: 'en', altLang: 'vi', altPath: '/vi/' },
  { path: '/faq/', lang: 'en', altLang: 'vi', altPath: '/vi/faq/' },
  { path: '/guides/', lang: 'en', altLang: 'vi', altPath: '/vi/guides/' },
  // EN guide detail
  { path: '/guides/how-it-works/', lang: 'en', altLang: 'vi', altPath: '/vi/guides/how-it-works/' },
  {
    path: '/guides/sheet-formats/',
    lang: 'en',
    altLang: 'vi',
    altPath: '/vi/guides/sheet-formats/',
  },
  {
    path: '/guides/windows-setup/',
    lang: 'en',
    altLang: 'vi',
    altPath: '/vi/guides/windows-setup/',
  },
  {
    path: '/guides/timing-engine/',
    lang: 'en',
    altLang: 'vi',
    altPath: '/vi/guides/timing-engine/',
  },
  {
    path: '/guides/security-boundaries/',
    lang: 'en',
    altLang: 'vi',
    altPath: '/vi/guides/security-boundaries/',
  },
  {
    path: '/guides/troubleshooting/',
    lang: 'en',
    altLang: 'vi',
    altPath: '/vi/guides/troubleshooting/',
  },
  // VI core
  { path: '/vi/', lang: 'vi', altLang: 'en', altPath: '/' },
  { path: '/vi/faq/', lang: 'vi', altLang: 'en', altPath: '/faq/' },
  { path: '/vi/guides/', lang: 'vi', altLang: 'en', altPath: '/guides/' },
  // VI guide detail
  { path: '/vi/guides/how-it-works/', lang: 'vi', altLang: 'en', altPath: '/guides/how-it-works/' },
  {
    path: '/vi/guides/sheet-formats/',
    lang: 'vi',
    altLang: 'en',
    altPath: '/guides/sheet-formats/',
  },
  {
    path: '/vi/guides/windows-setup/',
    lang: 'vi',
    altLang: 'en',
    altPath: '/guides/windows-setup/',
  },
  {
    path: '/vi/guides/timing-engine/',
    lang: 'vi',
    altLang: 'en',
    altPath: '/guides/timing-engine/',
  },
  {
    path: '/vi/guides/security-boundaries/',
    lang: 'vi',
    altLang: 'en',
    altPath: '/guides/security-boundaries/',
  },
  {
    path: '/vi/guides/troubleshooting/',
    lang: 'vi',
    altLang: 'en',
    altPath: '/guides/troubleshooting/',
  },
];

/** Compatibility aliases — must exist in dist, must NOT be in sitemap, must have noindex. */
export const COMPATIBILITY_ALIASES = ['/faq.html', '/vi/faq.html'];

/** Guide slugs — derived from CANONICAL_ROUTES, used for pair validation. */
export const GUIDE_SLUGS = [
  'how-it-works',
  'sheet-formats',
  'windows-setup',
  'timing-engine',
  'security-boundaries',
  'troubleshooting',
];

/**
 * Convert a canonical route path to its expected dist HTML file path (relative to dist/).
 *   '/'               → 'index.html'
 *   '/faq/'           → 'faq/index.html'
 *   '/guides/foo/'    → 'guides/foo/index.html'
 */
export function routeToDistHtml(routePath) {
  if (routePath === '/') return 'index.html';
  return `${routePath.replace(/^\/|\/$/g, '')}/index.html`;
}

import { existsSync, statSync, readdirSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  CANONICAL_ROUTES,
  COMPATIBILITY_ALIASES,
  routeToDistHtml,
  PRODUCTION_ORIGIN,
  BASE,
} from './canonical-routes.mjs';

const scriptDir = fileURLToPath(new URL('.', import.meta.url));
const dist = resolve(scriptDir, '..', 'dist');

// ── Required static files (non-HTML artifacts) ──────────────────────────────
const requiredStaticFiles = [
  'robots.txt',
  'llms.txt',
  'google40b614dcfbf81da7.html',
  'sitemap-index.xml',
  'favicon.ico',
  'favicon.svg',
  'assets/sky-auto-player-mark.svg',
  'assets/og-banner.jpg',
  'assets/images/picker.webp',
  'assets/images/picker-mobile.webp',
];

for (const relativePath of requiredStaticFiles) {
  if (!existsSync(resolve(dist, relativePath))) {
    throw new Error(`Missing required dist file: ${relativePath}`);
  }
}

// ── All 18 canonical HTML routes — hard contract ────────────────────────────
for (const route of CANONICAL_ROUTES) {
  const relativePath = routeToDistHtml(route.path);
  if (!existsSync(resolve(dist, relativePath))) {
    throw new Error(`Missing canonical route HTML: ${relativePath} (for ${route.path})`);
  }
}

// ── Compatibility aliases ───────────────────────────────────────────────────
for (const alias of COMPATIBILITY_ALIASES) {
  const relativePath = alias.replace(/^\//, '');
  if (!existsSync(resolve(dist, relativePath))) {
    throw new Error(`Missing compatibility alias: ${relativePath}`);
  }
}

for (const image of ['assets/images/picker.webp', 'assets/images/picker-mobile.webp']) {
  const bytes = statSync(resolve(dist, image)).size;
  if (bytes > 400_000) {
    throw new Error('Image budget exceeded for ' + image + ': ' + bytes + ' bytes');
  }
}

// ── HTML content checks: no localhost, correct base URL, no bare asset paths ─
const allHtmlRelativePaths = [
  ...CANONICAL_ROUTES.map((r) => routeToDistHtml(r.path)),
  ...COMPATIBILITY_ALIASES.map((a) => a.replace(/^\//, '')),
];

for (const relativePath of allHtmlRelativePaths) {
  const html = await readFile(resolve(dist, relativePath), 'utf8');
  if (html.includes('localhost')) {
    throw new Error(`Development URL found in ${relativePath}`);
  }
  if (!html.includes(`${PRODUCTION_ORIGIN}${BASE}/`)) {
    throw new Error(`Production canonical/base URL missing in ${relativePath}`);
  }
  for (const match of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    const value = match[1];
    if (value.startsWith('/assets/') || value.startsWith('/_astro/')) {
      throw new Error(`Base-less asset URL found in ${relativePath}: ${value}`);
    }
  }
}

// ── Google verification file ────────────────────────────────────────────────
const verification = await readFile(resolve(dist, 'google40b614dcfbf81da7.html'), 'utf8');
if (!verification.includes('google-site-verification: google40b614dcfbf81da7.html')) {
  throw new Error('Google verification file has unexpected content');
}

// ── Sitemap: must contain all 18 canonical routes, must exclude aliases ──────
const sitemapIndex = await readFile(resolve(dist, 'sitemap-index.xml'), 'utf8');
if (!sitemapIndex.includes('sitemap-0.xml')) {
  throw new Error('sitemap-index.xml does not reference sitemap-0.xml');
}

const sitemap = await readFile(resolve(dist, 'sitemap-0.xml'), 'utf8');
for (const route of CANONICAL_ROUTES) {
  const expectedLoc = `${PRODUCTION_ORIGIN}${BASE}${route.path}`;
  if (!sitemap.includes(`<loc>${expectedLoc}</loc>`)) {
    throw new Error(`Missing canonical route in sitemap: ${expectedLoc}`);
  }
}
for (const alias of COMPATIBILITY_ALIASES) {
  const aliasLoc = `${PRODUCTION_ORIGIN}${BASE}${alias}`;
  if (sitemap.includes(aliasLoc)) {
    throw new Error(`Alias must not appear in sitemap: ${aliasLoc}`);
  }
}

// ── HTML size sanity: no page should exceed 200 KB ─────────────────────────
function* walkHtml(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = resolve(dir, entry.name);
    if (entry.isDirectory()) yield* walkHtml(full);
    else if (entry.name === 'index.html') yield full;
  }
}
const HTML_MAX_BYTES = 200_000;
let htmlOverBudget = 0;
for (const htmlFile of walkHtml(dist)) {
  const bytes = statSync(htmlFile).size;
  if (bytes > HTML_MAX_BYTES) {
    console.warn(`[warn] HTML over ${HTML_MAX_BYTES / 1000}KB: ${htmlFile} (${bytes} bytes)`);
    htmlOverBudget++;
  }
}
if (htmlOverBudget > 0) {
  throw new Error(`${htmlOverBudget} HTML file(s) exceed the ${HTML_MAX_BYTES / 1000}KB budget`);
}

console.log(
  `dist verification passed — ${CANONICAL_ROUTES.length} canonical routes, ${requiredStaticFiles.length} static files.`,
);

import { existsSync, statSync, readdirSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = fileURLToPath(new URL('.', import.meta.url));
const dist = resolve(scriptDir, '..', 'dist');
const requiredFiles = [
  'index.html',
  'faq/index.html',
  'vi/index.html',
  'vi/faq/index.html',
  'faq.html',
  'vi/faq.html',
  'guides/index.html',
  'vi/guides/index.html',
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

for (const relativePath of requiredFiles) {
  if (!existsSync(resolve(dist, relativePath))) {
    throw new Error(`Missing required dist file: ${relativePath}`);
  }
}

for (const image of ['assets/images/picker.webp', 'assets/images/picker-mobile.webp']) {
  const bytes = statSync(resolve(dist, image)).size;
  if (bytes > 400_000) {
    throw new Error('Image budget exceeded for ' + image + ': ' + bytes + ' bytes');
  }
}

const htmlFiles = requiredFiles.filter(
  (file) => file.endsWith('.html') && file !== 'google40b614dcfbf81da7.html',
);
for (const relativePath of htmlFiles) {
  const html = await readFile(resolve(dist, relativePath), 'utf8');
  if (html.includes('localhost')) {
    throw new Error(`Development URL found in ${relativePath}`);
  }
  if (!html.includes('https://pumni.github.io/Sky-Auto-Player/')) {
    throw new Error(`Production canonical/base URL missing in ${relativePath}`);
  }
  for (const match of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    const value = match[1];
    if (value.startsWith('/assets/') || value.startsWith('/_astro/')) {
      throw new Error(`Base-less asset URL found in ${relativePath}: ${value}`);
    }
  }
}

const verification = await readFile(resolve(dist, 'google40b614dcfbf81da7.html'), 'utf8');
if (!verification.includes('google-site-verification: google40b614dcfbf81da7.html')) {
  throw new Error('Google verification file has unexpected content');
}
const sitemapIndex = await readFile(resolve(dist, 'sitemap-index.xml'), 'utf8');
if (!sitemapIndex.includes('sitemap-0.xml')) {
  throw new Error('sitemap-index.xml does not reference sitemap-0.xml');
}

const sitemap = await readFile(resolve(dist, 'sitemap-0.xml'), 'utf8');
const canonicalRoutes = [
  'https://pumni.github.io/Sky-Auto-Player/',
  'https://pumni.github.io/Sky-Auto-Player/faq/',
  'https://pumni.github.io/Sky-Auto-Player/vi/',
  'https://pumni.github.io/Sky-Auto-Player/vi/faq/',
  'https://pumni.github.io/Sky-Auto-Player/guides/',
  'https://pumni.github.io/Sky-Auto-Player/vi/guides/',
];
for (const route of canonicalRoutes) {
  if (!sitemap.includes(`<loc>${route}</loc>`)) {
    throw new Error(`Missing canonical route in sitemap: ${route}`);
  }
}

// HTML size sanity: no page should exceed 200 KB
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

console.log(`dist verification passed (${requiredFiles.length} required files).`);

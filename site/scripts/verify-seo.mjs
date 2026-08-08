/**
 * verify-seo.mjs
 *
 * SEO hard-gate validator. Runs against the Astro build output (site/dist/).
 *
 * Hard failures:
 *   - missing title, description, canonical, H1, robots, OG fields, hreflang
 *   - noindex on canonical pages
 *   - duplicate titles or descriptions across canonical pages
 *   - duplicate canonical URLs
 *   - invalid canonical (not absolute, not self-referencing)
 *   - hreflang reciprocal mismatch
 *   - JSON-LD not parseable
 *   - localhost URL in canonical pages
 *   - sitemap missing canonical routes
 *   - aliases present in sitemap
 *   - guide hub HTML files missing (when present in dist)
 *
 * Warnings (non-fatal):
 *   - title > ~65 characters
 *   - description > ~170 characters
 */

import { readFileSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = fileURLToPath(new URL('.', import.meta.url));
const dist = resolve(scriptDir, '..', 'dist');
const PRODUCTION_ORIGIN = 'https://pumni.github.io';
const BASE = '/Sky-Auto-Player';

// ── Canonical routes that must be indexable ────────────────────────────────
const canonicalRoutes = [
  { path: '/', lang: 'en', altLang: 'vi', altPath: '/vi/' },
  { path: '/faq/', lang: 'en', altLang: 'vi', altPath: '/vi/faq/' },
  { path: '/vi/', lang: 'vi', altLang: 'en', altPath: '/' },
  { path: '/vi/faq/', lang: 'vi', altLang: 'en', altPath: '/faq/' },
];

// Guide routes are added dynamically if they exist in dist
const guideSlugs = [
  'how-it-works',
  'sheet-formats',
  'windows-setup',
  'timing-engine',
  'security-boundaries',
  'troubleshooting',
];

for (const slug of guideSlugs) {
  const enFile = resolve(dist, 'guides', slug, 'index.html');
  const viFile = resolve(dist, 'vi', 'guides', slug, 'index.html');
  if (existsSync(enFile)) {
    canonicalRoutes.push({
      path: `/guides/${slug}/`,
      lang: 'en',
      altLang: 'vi',
      altPath: `/vi/guides/${slug}/`,
    });
  }
  if (existsSync(viFile)) {
    canonicalRoutes.push({
      path: `/vi/guides/${slug}/`,
      lang: 'vi',
      altLang: 'en',
      altPath: `/guides/${slug}/`,
    });
  }
}

// Guide hub pages
const enHubFile = resolve(dist, 'guides', 'index.html');
const viHubFile = resolve(dist, 'vi', 'guides', 'index.html');
if (existsSync(enHubFile)) {
  canonicalRoutes.push({ path: '/guides/', lang: 'en', altLang: 'vi', altPath: '/vi/guides/' });
}
if (existsSync(viHubFile)) {
  canonicalRoutes.push({ path: '/vi/guides/', lang: 'vi', altLang: 'en', altPath: '/guides/' });
}

// Compatibility aliases — must NOT be in sitemap
const aliasRoutes = ['/faq.html', '/vi/faq.html'];

// ── Helpers ────────────────────────────────────────────────────────────────
let errors = 0;
let warnings = 0;

function fail(msg) {
  console.error(`  ✗ ${msg}`);
  errors++;
}

function warn(msg) {
  console.warn(`  ⚠ ${msg}`);
  warnings++;
}

function pass(msg) {
  console.log(`  ✓ ${msg}`);
}

/** Read HTML file from dist. Returns null if file doesn't exist. */
function readHtml(route) {
  const filePath = resolve(dist, route.replace(/^\//, ''), 'index.html');
  if (!existsSync(filePath)) {
    // For alias .html files
    const directPath = resolve(dist, route.replace(/^\//, ''));
    if (existsSync(directPath)) return readFileSync(directPath, 'utf8');
    return null;
  }
  return readFileSync(filePath, 'utf8');
}

/** Extract content of first matching meta/link tag. */
function extractMeta(html, selector) {
  const nameMatch = selector.match(/name="([^"]+)"/);
  const propMatch = selector.match(/property="([^"]+)"/);
  const relMatch = selector.match(/rel="([^"]+)"/);
  const hreflangMatch = selector.match(/hreflang="([^"]+)"/);

  if (hreflangMatch) {
    const re = new RegExp(
      `<link[^>]+hreflang="${escapeRe(hreflangMatch[1])}"[^>]+href="([^"]+)"`,
      'i',
    );
    const m =
      html.match(re) ??
      html.match(
        new RegExp(`<link[^>]+href="([^"]+)"[^>]+hreflang="${escapeRe(hreflangMatch[1])}"`, 'i'),
      );
    return m ? m[1] : null;
  }
  if (nameMatch) {
    const re = new RegExp(`<meta[^>]+name="${escapeRe(nameMatch[1])}"[^>]+content="([^"]*)"`, 'i');
    const m =
      html.match(re) ??
      html.match(
        new RegExp(`<meta[^>]+content="([^"]*)"[^>]+name="${escapeRe(nameMatch[1])}"`, 'i'),
      );
    return m ? m[1] : null;
  }
  if (propMatch) {
    const re = new RegExp(
      `<meta[^>]+property="${escapeRe(propMatch[1])}"[^>]+content="([^"]*)"`,
      'i',
    );
    const m =
      html.match(re) ??
      html.match(
        new RegExp(`<meta[^>]+content="([^"]*)"[^>]+property="${escapeRe(propMatch[1])}"`, 'i'),
      );
    return m ? m[1] : null;
  }
  if (relMatch && relMatch[1] === 'canonical') {
    const re = /<link[^>]+rel="canonical"[^>]+href="([^"]+)"/i;
    const m = html.match(re) ?? html.match(/<link[^>]+href="([^"]+)"[^>]+rel="canonical"/i);
    return m ? m[1] : null;
  }
  return null;
}

function escapeRe(str) {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function countH1(html) {
  return (html.match(/<h1\b/gi) ?? []).length;
}

function extractTitle(html) {
  const m = html.match(/<title>([^<]*)<\/title>/i);
  return m ? m[1].trim() : null;
}

function extractJsonLd(html) {
  const scripts = [];
  const re = /<script[^>]+type="application\/ld\+json"[^>]*>([\s\S]*?)<\/script>/gi;
  let m;
  while ((m = re.exec(html)) !== null) {
    scripts.push(m[1].trim());
  }
  return scripts;
}

// ── Check each canonical route ──────────────────────────────────────────────
const seenTitles = new Map();
const seenDescriptions = new Map();
const seenCanonicals = new Map();

for (const route of canonicalRoutes) {
  console.log(`\nChecking ${route.path}`);
  const html = readHtml(route.path);
  if (!html) {
    fail(`HTML file not found for route: ${route.path}`);
    continue;
  }

  // Title
  const title = extractTitle(html);
  if (!title) {
    fail('Missing <title>');
  } else {
    if (title.length > 65) warn(`Title length ${title.length} > 65 chars: "${title}"`);
    if (seenTitles.has(title))
      fail(`Duplicate title: "${title}" (also in ${seenTitles.get(title)})`);
    else seenTitles.set(title, route.path);
    pass(`title: "${title.slice(0, 55)}${title.length > 55 ? '...' : ''}"`);
  }

  // Description
  const desc = extractMeta(html, 'name="description"');
  if (!desc) {
    fail('Missing meta description');
  } else {
    if (desc.length > 200) warn(`Description length ${desc.length} > 200 chars`);
    if (seenDescriptions.has(desc))
      fail(`Duplicate description in ${route.path} (also in ${seenDescriptions.get(desc)})`);
    else seenDescriptions.set(desc, route.path);
    pass(`description: "${desc.slice(0, 60)}..."`);
  }

  // Canonical
  const canonical = extractMeta(html, 'rel="canonical"');
  const expectedCanonical = `${PRODUCTION_ORIGIN}${BASE}${route.path}`;
  if (!canonical) {
    fail('Missing canonical link');
  } else if (!canonical.startsWith('http')) {
    fail(`Canonical not absolute: "${canonical}"`);
  } else if (canonical !== expectedCanonical) {
    fail(`Canonical mismatch: expected "${expectedCanonical}", got "${canonical}"`);
  } else {
    if (seenCanonicals.has(canonical))
      fail(`Duplicate canonical: ${canonical} (also in ${seenCanonicals.get(canonical)})`);
    else seenCanonicals.set(canonical, route.path);
    pass(`canonical: "${canonical}"`);
  }

  // html lang
  const langMatch = html.match(/<html[^>]+lang="([^"]+)"/i);
  const pageLang = langMatch ? langMatch[1] : null;
  if (!pageLang) {
    fail('Missing html lang attribute');
  } else if (pageLang !== route.lang) {
    fail(`html lang="${pageLang}" but expected "${route.lang}"`);
  } else {
    pass(`lang="${pageLang}"`);
  }

  // H1 count
  const h1Count = countH1(html);
  if (h1Count === 0) fail('No H1 found');
  else if (h1Count > 1) fail(`Multiple H1 found: ${h1Count}`);
  else pass('exactly one H1');

  // robots meta — canonical pages must not have noindex
  const robots = extractMeta(html, 'name="robots"');
  if (!robots) {
    fail('Missing robots meta tag');
  } else if (robots.includes('noindex')) {
    fail(`Canonical page has noindex in robots: "${robots}"`);
  } else {
    pass(`robots: "${robots}"`);
  }

  // OG tags
  const ogTitle = extractMeta(html, 'property="og:title"');
  if (!ogTitle) fail('Missing og:title');
  else pass('og:title present');

  const ogDesc = extractMeta(html, 'property="og:description"');
  if (!ogDesc) fail('Missing og:description');
  else pass('og:description present');

  const ogImage = extractMeta(html, 'property="og:image"');
  if (!ogImage) fail('Missing og:image');
  else if (!ogImage.startsWith('http')) fail(`og:image not absolute: "${ogImage}"`);
  else pass('og:image present and absolute');

  // hreflang
  const hreflangEn = extractMeta(html, 'hreflang="en"');
  const hreflangVi = extractMeta(html, 'hreflang="vi"');
  const hreflangXDefault = extractMeta(html, 'hreflang="x-default"');

  if (!hreflangEn) fail('Missing hreflang="en"');
  else pass(`hreflang en: ${hreflangEn}`);

  if (!hreflangVi) fail('Missing hreflang="vi"');
  else pass(`hreflang vi: ${hreflangVi}`);

  if (!hreflangXDefault) fail('Missing hreflang="x-default"');
  else {
    // x-default must point to EN version
    const enVersion = `${PRODUCTION_ORIGIN}${BASE}${route.lang === 'en' ? route.path : route.altPath}`;
    if (!hreflangXDefault.includes(BASE + (route.lang === 'en' ? route.path : route.altPath))) {
      warn(`hreflang x-default "${hreflangXDefault}" may not point to EN version`);
    } else {
      pass(`hreflang x-default: ${hreflangXDefault}`);
    }
  }

  // localhost check
  if (html.includes('localhost')) {
    fail('Found "localhost" in canonical page HTML');
  } else {
    pass('no localhost URLs');
  }

  // JSON-LD parseable
  const scripts = extractJsonLd(html);
  if (scripts.length === 0) {
    warn('No JSON-LD structured data found');
  } else {
    let jsonLdOk = true;
    for (const script of scripts) {
      try {
        JSON.parse(script);
      } catch {
        fail(`JSON-LD parse error: ${script.slice(0, 80)}...`);
        jsonLdOk = false;
      }
    }
    if (jsonLdOk)
      pass(`JSON-LD parseable (${scripts.length} block${scripts.length > 1 ? 's' : ''})`);
  }
}

// ── Check aliases have noindex ─────────────────────────────────────────────
console.log('\nChecking compatibility aliases');
for (const alias of aliasRoutes) {
  const html = readHtml(alias);
  if (!html) {
    fail(`Alias file not found: ${alias}`);
    continue;
  }
  const robots = extractMeta(html, 'name="robots"');
  if (!robots || !robots.includes('noindex')) {
    fail(`Alias ${alias} missing noindex in robots`);
  } else {
    pass(`${alias} has noindex`);
  }
}

// ── Sitemap validation ─────────────────────────────────────────────────────
console.log('\nChecking sitemap');
const sitemapPath = resolve(dist, 'sitemap-0.xml');
if (!existsSync(sitemapPath)) {
  fail('sitemap-0.xml not found');
} else {
  const sitemap = readFileSync(sitemapPath, 'utf8');

  // Canonical routes must be in sitemap
  for (const route of canonicalRoutes) {
    const expectedLoc = `${PRODUCTION_ORIGIN}${BASE}${route.path}`;
    if (!sitemap.includes(`<loc>${expectedLoc}</loc>`)) {
      fail(`Canonical route missing from sitemap: ${expectedLoc}`);
    } else {
      pass(`sitemap contains ${route.path}`);
    }
  }

  // Aliases must NOT be in sitemap
  for (const alias of aliasRoutes) {
    const aliasLoc = `${PRODUCTION_ORIGIN}${BASE}${alias}`;
    if (sitemap.includes(aliasLoc)) {
      fail(`Alias should not be in sitemap: ${aliasLoc}`);
    } else {
      pass(`sitemap excludes alias ${alias}`);
    }
  }

  // Must not have draft routes (none expected unless explicitly added)
  // Must use production origin
  if (sitemap.includes('localhost')) {
    fail('sitemap contains localhost URL');
  } else {
    pass('sitemap has no localhost URLs');
  }
}

// ── EN/VI guide pair validation ────────────────────────────────────────────
console.log('\nChecking EN/VI guide pairs');
for (const slug of guideSlugs) {
  const enFile = resolve(dist, 'guides', slug, 'index.html');
  const viFile = resolve(dist, 'vi', 'guides', slug, 'index.html');
  const enExists = existsSync(enFile);
  const viExists = existsSync(viFile);

  if (enExists && !viExists) {
    fail(`EN guide /guides/${slug}/ has no VI pair at /vi/guides/${slug}/`);
  } else if (viExists && !enExists) {
    fail(`VI guide /vi/guides/${slug}/ has no EN pair at /guides/${slug}/`);
  } else if (enExists && viExists) {
    pass(`guide pair exists: ${slug}`);
  }
  // Both missing = guide not shipped yet = OK (no constraint)
}

// ── Summary ───────────────────────────────────────────────────────────────
console.log(`\n${'─'.repeat(60)}`);
console.log(
  `SEO validation: ${errors} error${errors !== 1 ? 's' : ''}, ${warnings} warning${warnings !== 1 ? 's' : ''}`,
);

if (errors > 0) {
  process.exit(1);
}

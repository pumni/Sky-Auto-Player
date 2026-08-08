/**
 * verify-seo.mjs
 *
 * SEO hard-gate validator. Runs against the Astro build output (site/dist/).
 *
 * Hard failures:
 *   - missing canonical route (all 18 required unconditionally via canonical-routes.mjs)
 *   - missing title, description, canonical, H1, robots, OG fields, hreflang
 *   - noindex on canonical pages
 *   - duplicate titles or descriptions across canonical pages
 *   - duplicate canonical URLs
 *   - invalid canonical (not absolute, not self-referencing)
 *   - hreflang reciprocal mismatch
 *   - hreflang x-default not pointing to EN version
 *   - JSON-LD not parseable
 *   - localhost URL in canonical pages
 *   - sitemap missing canonical routes
 *   - aliases present in sitemap
 *   - forbidden unsupported timing claims in homepage HTML
 *
 * Warnings (non-fatal):
 *   - title > ~65 characters
 *   - description > ~200 characters
 */

import { readFileSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  CANONICAL_ROUTES,
  COMPATIBILITY_ALIASES,
  GUIDE_SLUGS,
  routeToDistHtml,
  PRODUCTION_ORIGIN,
  BASE,
} from './canonical-routes.mjs';

const scriptDir = fileURLToPath(new URL('.', import.meta.url));
const dist = resolve(scriptDir, '..', 'dist');

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

/** Read HTML file from dist given a route path or alias path. Returns null if missing. */
function readHtml(routePath) {
  // Alias paths like /faq.html
  if (routePath.endsWith('.html')) {
    const directPath = resolve(dist, routePath.replace(/^\//, ''));
    return existsSync(directPath) ? readFileSync(directPath, 'utf8') : null;
  }
  const filePath = resolve(dist, routeToDistHtml(routePath));
  return existsSync(filePath) ? readFileSync(filePath, 'utf8') : null;
}

function escapeRe(str) {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
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

// ── Claim regression guard ────────────────────────────────────────────────
// Scope: only homepage (dist/index.html and dist/vi/index.html).
// These phrases are marketing claims that exceed current evidence boundary.
// Does NOT apply to guide content or technical docs discussing WHY claims are not made.
const FORBIDDEN_CLAIM_PATTERNS = [
  { phrase: 'under 1 millisecond', note: 'unsupported universal sub-ms claim' },
  { phrase: 'sub-millisecond', note: 'unsupported universal sub-ms claim' },
  { phrase: 'same game frame', note: 'unsupported in-game frame guarantee' },
  { phrase: 'moment it actually lands', note: 'unsupported actual-landing-time claim' },
  { phrase: "aligned to the game's frames", note: 'unsupported in-game frame alignment claim' },
  { phrase: 'aligned to the game', note: 'unsupported in-game alignment claim' },
  { phrase: 'frame-aligned', note: 'unsupported frame-aligned claim' },
  { phrase: 'SAME FRAME', note: 'unsupported in-game frame claim in diagram' },
  // VI equivalents
  { phrase: 'lệch dưới 1 phần nghìn', note: 'unsupported universal sub-ms claim (VI)' },
  { phrase: 'rơi cùng một khung hình', note: 'unsupported in-game frame guarantee (VI)' },
  { phrase: 'thực sự vang lên', note: 'unsupported actual-audio-onset claim (VI)' },
  { phrase: 'cùng một khung hình của game', note: 'unsupported in-game frame guarantee (VI)' },
  { phrase: 'căn theo khung hình game', note: 'unsupported frame alignment claim (VI)' },
  { phrase: 'căn theo khung hình của game', note: 'unsupported frame alignment claim (VI)' },
  { phrase: 'CÙNG FRAME', note: 'unsupported in-game frame claim in diagram (VI)' },
];

const HOMEPAGE_PATHS = ['/', '/vi/'];
console.log('\nChecking claim regression guard (homepage only)');
for (const homePath of HOMEPAGE_PATHS) {
  const html = readHtml(homePath);
  if (!html) {
    fail(`Homepage HTML not found: ${homePath}`);
    continue;
  }
  for (const { phrase, note } of FORBIDDEN_CLAIM_PATTERNS) {
    if (html.includes(phrase)) {
      fail(`Forbidden claim in ${homePath}: "${phrase}" — ${note}`);
    }
  }
  pass(`${homePath} claim regression guard passed`);
}

// ── Check each canonical route ──────────────────────────────────────────────
const seenTitles = new Map();
const seenDescriptions = new Map();
const seenCanonicals = new Map();

for (const route of CANONICAL_ROUTES) {
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

  // hreflang — exact URL comparison
  const hreflangEn = extractMeta(html, 'hreflang="en"');
  const hreflangVi = extractMeta(html, 'hreflang="vi"');
  const hreflangXDefault = extractMeta(html, 'hreflang="x-default"');

  // Expected EN and VI absolute URLs
  const enAbsUrl = `${PRODUCTION_ORIGIN}${BASE}${route.lang === 'en' ? route.path : route.altPath}`;
  const viAbsUrl = `${PRODUCTION_ORIGIN}${BASE}${route.lang === 'vi' ? route.path : route.altPath}`;

  if (!hreflangEn) fail('Missing hreflang="en"');
  else if (hreflangEn !== enAbsUrl)
    fail(`hreflang en mismatch: got "${hreflangEn}", expected "${enAbsUrl}"`);
  else pass(`hreflang en: ${hreflangEn}`);

  if (!hreflangVi) fail('Missing hreflang="vi"');
  else if (hreflangVi !== viAbsUrl)
    fail(`hreflang vi mismatch: got "${hreflangVi}", expected "${viAbsUrl}"`);
  else pass(`hreflang vi: ${hreflangVi}`);

  if (!hreflangXDefault) {
    fail('Missing hreflang="x-default"');
  } else if (hreflangXDefault !== enAbsUrl) {
    fail(`hreflang x-default must equal EN URL "${enAbsUrl}", got "${hreflangXDefault}"`);
  } else {
    pass(`hreflang x-default: ${hreflangXDefault}`);
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
for (const alias of COMPATIBILITY_ALIASES) {
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

  for (const route of CANONICAL_ROUTES) {
    const expectedLoc = `${PRODUCTION_ORIGIN}${BASE}${route.path}`;
    if (!sitemap.includes(`<loc>${expectedLoc}</loc>`)) {
      fail(`Canonical route missing from sitemap: ${expectedLoc}`);
    } else {
      pass(`sitemap contains ${route.path}`);
    }
  }

  for (const alias of COMPATIBILITY_ALIASES) {
    const aliasLoc = `${PRODUCTION_ORIGIN}${BASE}${alias}`;
    if (sitemap.includes(aliasLoc)) {
      fail(`Alias should not be in sitemap: ${aliasLoc}`);
    } else {
      pass(`sitemap excludes alias ${alias}`);
    }
  }

  if (sitemap.includes('localhost')) {
    fail('sitemap contains localhost URL');
  } else {
    pass('sitemap has no localhost URLs');
  }
}

// ── EN/VI guide pair validation (hard contract — both must exist) ──────────
console.log('\nChecking EN/VI guide pairs');
for (const slug of GUIDE_SLUGS) {
  const enFile = resolve(dist, 'guides', slug, 'index.html');
  const viFile = resolve(dist, 'vi', 'guides', slug, 'index.html');
  const enExists = existsSync(enFile);
  const viExists = existsSync(viFile);

  if (!enExists && !viExists) {
    fail(`Guide pair missing entirely: ${slug} (both EN and VI required)`);
  } else if (enExists && !viExists) {
    fail(`EN guide /guides/${slug}/ has no VI pair at /vi/guides/${slug}/`);
  } else if (viExists && !enExists) {
    fail(`VI guide /vi/guides/${slug}/ has no EN pair at /guides/${slug}/`);
  } else {
    pass(`guide pair exists: ${slug}`);
  }
}

// ── Related guides contract validation ────────────────────────────────────
console.log('\nChecking related guides contract');
for (const slug of GUIDE_SLUGS) {
  for (const langPrefix of ['', 'vi/']) {
    const pageRoute = `/${langPrefix}guides/${slug}/`;
    const html = readHtml(pageRoute);
    if (!html) continue;

    const matches = [...html.matchAll(/href="[^"]*?\/(?:vi\/)?guides\/([^/]+)\/"/gi)];
    const relatedSlugs = matches
      .map((m) => m[1])
      .filter((s) => s !== slug && s !== '' && GUIDE_SLUGS.includes(s));
    const uniqueSlugs = new Set(relatedSlugs);

    if (uniqueSlugs.size < 2 || uniqueSlugs.size > 3) {
      fail(
        `Guide page ${pageRoute} has ${uniqueSlugs.size} valid related guide links, expected 2–3`,
      );
    } else {
      pass(`${pageRoute} has ${uniqueSlugs.size} valid related guide links`);
    }
  }
}

// ── Summary ───────────────────────────────────────────────────────────────
console.log(`\n${'─'.repeat(60)}`);
console.log(
  `SEO validation: ${errors} error${errors !== 1 ? 's' : ''}, ${warnings} warning${warnings !== 1 ? 's' : ''}`,
);

if (errors > 0) {
  process.exit(1);
}

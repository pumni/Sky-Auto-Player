# Plan: Sky Auto Player — SEO/GEO Growth Architecture 2026

> **Status:** IMPLEMENTED — ACCEPTED / SHIPPED (Verified on HEAD `7c48b3ed82f8eda979c7eabbff9aec1a35e09598`, GitHub Pages deployment run #30 `31279125167`)
> **Date:** 2026-08-08 (Accepted 2026-08-09)
> **Repository:** `pumni/Sky-Auto-Player`
> **Primary scope:** `site/` GitHub Pages marketing website
> **Implemented by:** Antigravity AI coding agent

This is a working plan / proposal document. It is NOT a normative architecture document.
`AGENTS.md`, `SECURITY.md`, and P2 normative documents always take priority over this plan.

---

## Objective

Increase discoverability, citation potential, organic interaction, and qualified traffic from
Google Search, Google AI Overviews / AI Mode, Bing/Copilot-style search, and ChatGPT Search
without spam, doorway pages, fake authority signals, or GEO hacks.

---

## Implementation Summary

### PR 1 — SEO Foundation

Files changed:
- `site/src/layouts/BaseLayout.astro` — expanded Props, robots meta, dynamic og:type, array structuredData
- `site/src/pages/index.astro` — @graph schema (WebSite + SoftwareApplication), APP_VERSION import
- `site/src/pages/vi/index.astro` — @graph schema VI
- `site/src/data/home.en.ts` — entity disambiguation kicker, affiliation disclaimer, guides nav
- `site/src/data/home.vi.ts` — VI equivalents
- `site/src/data/home.types.ts` — affiliationDisclaimer field, guides nav field
- `site/src/components/home/Hero.astro` — render affiliationDisclaimer
- `site/public/faq.html` — noindex added
- `site/public/vi/faq.html` — noindex added
- `site/scripts/sync-version.mjs` — NEW prebuild script reading pyproject.toml version
- `site/scripts/verify-seo.mjs` — NEW SEO validator (CI hard gate)
- `site/package.json` — prebuild + verify:seo scripts
- `site/.gitignore` — src/generated/ excluded
- `.github/actions/site-validate/action.yml` — verify:seo step added

### PR 2 — Guide Platform

Files changed:
- `site/src/content.config.ts` — guides collection schema with generateId
- `site/src/layouts/GuideLayout.astro` — NEW (Article + BreadcrumbList JSON-LD)
- `site/src/components/guide/GuideBreadcrumb.astro` — NEW
- `site/src/components/guide/RelatedGuides.astro` — NEW
- `site/src/components/guide/GuideHub.astro` — NEW
- `site/src/pages/guides/index.astro` — NEW EN guide hub
- `site/src/pages/guides/[slug].astro` — NEW EN dynamic guide route
- `site/src/pages/vi/guides/index.astro` — NEW VI guide hub
- `site/src/pages/vi/guides/[slug].astro` — NEW VI dynamic guide route
- `site/src/components/layout/SiteHeader.astro` — Guides nav link
- `site/src/components/layout/SiteFooter.astro` — Guides footer link
- `site/scripts/verify-dist.mjs` — guide hub files added
- `site/scripts/verify-seo.mjs` — pair validation added
- `site/tests/e2e/accessibility.spec.ts` — stale row count corrected

### PR 3 — First-Party Content (12 guides)

Files added: `site/src/content/guides/en/` + `site/src/content/guides/vi/`

| Slug | Category | Evidence |
|---|---|---|
| how-it-works | getting-started | architecture.md, SECURITY.md, README.md, rt-dispatch-architecture.md |
| sheet-formats | getting-started | README.md, domain/ source |
| windows-setup | getting-started | README.md, distribution-and-update.md |
| timing-engine | playback-timing | timing-principles.md, hold-frame-model.md, rt-dispatch-architecture.md, README.md |
| security-boundaries | technical-safety | SECURITY.md, audit script, platform/, updater.ps1 |
| troubleshooting | support | README.md, timing-principles.md, distribution-and-update.md |

### PR 4 — Quality (this PR)

Files changed:
- `site/src/pages/llms.txt.ts` — guide URLs added, affiliation notice
- `docs/plan/2026-08-08_pages-geo-seo-growth-plan.md` — this file
- `docs/INDEX.md` — entry added

---

## Canonical Routes (18 total)

```
/                              EN homepage
/faq/                          EN FAQ
/guides/                       EN guide hub
/guides/how-it-works/
/guides/sheet-formats/
/guides/windows-setup/
/guides/timing-engine/
/guides/security-boundaries/
/guides/troubleshooting/

/vi/                           VI homepage
/vi/faq/                       VI FAQ
/vi/guides/                    VI guide hub
/vi/guides/how-it-works/
/vi/guides/sheet-formats/
/vi/guides/windows-setup/
/vi/guides/timing-engine/
/vi/guides/security-boundaries/
/vi/guides/troubleshooting/
```

Compatibility aliases (NOT in sitemap, have noindex):
```
/faq.html
/vi/faq.html
```

---

## Acceptance Gates (all green)

```
bun run check          # 0 errors
bun run lint           # 0 issues
bun run format:check   # all formatted
bun run build          # 18 canonical pages built
bun run verify:dist    # 18 required files present
bun run verify:seo     # 0 errors, 0 warnings
bun run test:functional# 0 failures (Functional E2E + Accessibility)
bun run test:visual    # 0 failures (Visual regression default snapshots)
```

---

## Constraints Respected

- No spam, no doorway pages, no fake authority signals
- No SEO-driven telemetry or tracking
- All guide content backed by evidence links to source repository
- No universal timing guarantees (scoped correctly per README)
- No affiliation claim with thatgamecompany (explicit disclaimer on homepage and in guides)
- Security mandates P0 untouched — no `src/`, `rust/`, or application code modified
- All normative docs (AGENTS.md P2) unchanged

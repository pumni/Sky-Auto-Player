import { test, expect } from '@playwright/test';

const origin = 'http://localhost:4321/Sky-Auto-Player';

const canonicalRoutes = [
  { path: '/', lang: 'en', canonical: 'https://pumni.github.io/Sky-Auto-Player/' },
  { path: '/faq/', lang: 'en', canonical: 'https://pumni.github.io/Sky-Auto-Player/faq/' },
  { path: '/vi/', lang: 'vi', canonical: 'https://pumni.github.io/Sky-Auto-Player/vi/' },
  { path: '/vi/faq/', lang: 'vi', canonical: 'https://pumni.github.io/Sky-Auto-Player/vi/faq/' },
] as const;

test.describe('published route and asset contracts', () => {
  for (const route of canonicalRoutes) {
    test(route.path + ' returns canonical HTML', async ({ request }) => {
      const response = await request.get(origin + route.path);
      expect(response.status()).toBe(200);

      const html = await response.text();
      expect(html).toContain('<html lang="' + route.lang + '">');
      expect(html).toContain('<main');
      expect(html).toMatch(/<h1\b/);
      expect(html).toContain('<link rel="canonical" href="' + route.canonical + '"');
      expect(html).not.toContain('localhost');
    });
  }

  test('legacy FAQ URLs retain redirect documents', async ({ request }) => {
    const redirects = [
      {
        path: '/faq.html',
        canonical: 'https://pumni.github.io/Sky-Auto-Player/faq/',
        link: './faq/',
      },
      {
        path: '/vi/faq.html',
        canonical: 'https://pumni.github.io/Sky-Auto-Player/vi/faq/',
        link: './faq/',
      },
    ];

    for (const redirect of redirects) {
      const response = await request.get(origin + redirect.path);
      expect(response.status()).toBe(200);
      const html = await response.text();
      expect(html).toContain('<link rel="canonical" href="' + redirect.canonical + '"');
      expect(html).toContain('url=' + redirect.link);
      expect(html).toContain('href="' + redirect.link + '"');
    }
  });

  test('text endpoints and verification file are publishable', async ({ request }) => {
    const robots = await request.get(origin + '/robots.txt');
    expect(robots.status()).toBe(200);
    expect(await robots.text()).toContain(
      'Sitemap: https://pumni.github.io/Sky-Auto-Player/sitemap-index.xml',
    );

    const llms = await request.get(origin + '/llms.txt');
    expect(llms.status()).toBe(200);
    const llmsText = await llms.text();
    expect(llmsText).toContain('Sky Auto Player');
    expect(llmsText).toContain('SendInput API only');

    const google = await request.get(origin + '/google40b614dcfbf81da7.html');
    expect(google.status()).toBe(200);
    expect((await google.text()).trim()).toBe(
      'google-site-verification: google40b614dcfbf81da7.html',
    );
  });

  test('critical assets return 200 and home local URLs are base-aware', async ({ request }) => {
    const assets = [
      '/favicon.ico',
      '/favicon.svg',
      '/favicon-16x16.png',
      '/favicon-32x32.png',
      '/apple-touch-icon.png',
      '/assets/sky-auto-player-mark.svg',
      '/assets/sky-auto-player-mark-mono.svg',
      '/assets/sky-auto-player-mark-no-bg.svg',
      '/assets/og-banner.jpg',
      '/assets/images/library-real-tauri.png',
      '/assets/images/minimum-real-tauri.png',
      '/assets/images/detail-real-tauri.png',
      '/assets/images/settings-real-tauri.png',
      '/assets/images/og-banner.jpg',
    ];

    for (const asset of assets) {
      const response = await request.get(origin + asset);
      expect(response.status(), asset).toBe(200);
    }

    const home = await request.get(origin + '/');
    const html = await home.text();
    expect(html).toContain('Canonical desktop interface');
    expect(html).toContain('/assets/images/library-real-tauri.png');
    expect(html).not.toContain('terminal picker');
    const localUrls = Array.from(
      html.matchAll(/(?:href|src)="(\/Sky-Auto-Player\/[^"#?]+)"/g),
      (match) => match[1],
    );

    for (const url of new Set(localUrls)) {
      const response = await request.get('http://localhost:4321' + url);
      expect(response.status(), url).toBe(200);
    }
  });
});

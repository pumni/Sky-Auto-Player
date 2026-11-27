import { test, expect } from '@playwright/test';

const origin = process.env.PLAYWRIGHT_BASE_URL;
if (!origin) {
  throw new Error(
    'PLAYWRIGHT_BASE_URL is required. Run tests through "bun run test:e2e" or provide an explicit PLAYWRIGHT_BASE_URL.',
  );
}
const hostOrigin = new URL(origin).origin;
const officialReleasesUrl = 'https://github.com/pumni/Sky-Auto-Player/releases';
const stableReleaseUrl = `${officialReleasesUrl}/latest`;

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

  test('English and Vietnamese homepage JSON-LD links to the stable release without snapshot metadata', async ({
    request,
  }) => {
    for (const path of ['/', '/vi/']) {
      const response = await request.get(origin + path);
      expect(response.status(), path).toBe(200);

      const html = await response.text();
      const scripts = Array.from(
        html.matchAll(/<script[^>]+type="application\/ld\+json"[^>]*>([\s\S]*?)<\/script>/gi),
        (match) => JSON.parse(match[1]),
      );
      const softwareApplications = scripts
        .flatMap((block) => (Array.isArray(block['@graph']) ? block['@graph'] : [block]))
        .filter((entry) => entry && entry['@type'] === 'SoftwareApplication');

      expect(softwareApplications).toHaveLength(1);
      expect(softwareApplications[0]).not.toHaveProperty('softwareVersion');
      expect(softwareApplications[0].downloadUrl).toBe(stableReleaseUrl);
      expect(softwareApplications[0].operatingSystem).toBe('Windows 11 (64-bit)');
    }
  });

  test('localized product pages describe current OS, package trust, imports and timing limits', async ({
    request,
  }) => {
    const cases = [
      {
        path: '/',
        required: [
          'Windows 11 x64',
          'free, open-source music sheet player',
          'Download for Windows',
          'GitHub Releases',
          'current Windows user',
          'playlist',
          'Does not inspect game memory or change game files',
          '.exe.sig',
        ],
      },
      {
        path: '/vi/',
        required: [
          'Windows 11 64-bit',
          'miễn phí, mã nguồn mở',
          'Tải cho Windows',
          'GitHub Releases',
          'danh sách phát',
          'Không đọc bộ nhớ hoặc sửa tệp game',
          'Cài đặt cho tài khoản Windows hiện tại',
          'không có chữ ký Authenticode',
          '.exe.sig',
        ],
      },
      {
        path: '/faq/',
        required: [
          'Windows 11 64-bit only',
          'not Authenticode-signed',
          'not needed for a manual install',
        ],
      },
      {
        path: '/vi/faq/',
        required: [
          'Windows 11 64-bit',
          'chưa được ký Authenticode',
          'không cần cho cài đặt thủ công',
        ],
      },
      {
        path: '/guides/windows-setup/',
        required: ['intentionally unsigned for Authenticode', 'manual install'],
      },
      {
        path: '/vi/guides/windows-setup/',
        required: ['chưa được ký Authenticode', 'cài đặt thủ công'],
      },
      {
        path: '/guides/sheet-formats/',
        required: ['Create or select a playlist', 'reference the original files'],
      },
      {
        path: '/vi/guides/sheet-formats/',
        required: ['Tạo hoặc chọn playlist', 'tham chiếu file gốc'],
      },
    ];

    for (const entry of cases) {
      const response = await request.get(origin + entry.path);
      expect(response.status(), entry.path).toBe(200);
      const html = await response.text();
      for (const phrase of entry.required) expect(html, entry.path).toContain(phrase);
      if (entry.path === '/' || entry.path === '/vi/') {
        expect(html, entry.path).toMatch(/Windows 11 (?:x64|64-bit)/i);
        expect(html, entry.path).not.toContain('WINDOWS 10–11');
      }
      expect(html, entry.path).not.toContain('Reload songs');
      expect(html, entry.path).not.toContain('--tuning');
    }
  });

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

  test('stable download actions and release history have distinct destinations', async ({
    page,
  }) => {
    for (const root of ['/Sky-Auto-Player/', '/Sky-Auto-Player/vi/']) {
      await page.goto(root);
      await expect(page.locator('.header-download')).toHaveCount(0);
      for (const selector of ['.hero__primary', '.trust-download__button']) {
        await expect(page.locator(selector)).toHaveAttribute('href', stableReleaseUrl);
      }
      await expect(page.locator('footer a[href="' + officialReleasesUrl + '"]')).toHaveCount(1);
      await expect(page.locator('main')).not.toContainText(/\bv?\d+\.\d+\.\d+\b/);
      for (const route of ['faq/', 'guides/windows-setup/']) {
        await page.goto(root + route);
        await expect(page.locator('.header-download')).toHaveAttribute('href', stableReleaseUrl);
        expect(
          await page.locator('main a[href="' + stableReleaseUrl + '"]').count(),
        ).toBeGreaterThan(0);
        await expect(page.locator('footer a[href="' + officialReleasesUrl + '"]')).toHaveCount(1);
      }
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
    expect(llmsText).toContain(
      'SendInput API after fresh foreground and retained-target identity checks',
    );
    expect(llmsText).toContain('Bounded safety cleanup may separately attempt releases');
    expect(llmsText).toContain('Windows 11 x64');
    expect(llmsText).toContain('Tauri updater separately verifies the .exe.sig sidecar');
    expect(llmsText).toContain('sender-side');

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
      '/assets/images/desktop-library-browser-preview-1366x768.png',
      '/assets/images/desktop-song-detail-browser-preview-1366x768.png',
      '/assets/images/desktop-settings-playback-browser-preview-1366x768.png',
      '/assets/images/og-banner.jpg',
    ];

    for (const asset of assets) {
      const response = await request.get(origin + asset);
      expect(response.status(), asset).toBe(200);
    }

    const home = await request.get(origin + '/');
    const html = await home.text();
    expect(html).toContain('Browser mock preview');
    expect(html).toContain('/assets/images/desktop-library-browser-preview-1366x768.png');
    expect(html).toContain('not a native capture');
    expect(html).not.toContain('terminal picker');
    const localUrls = Array.from(
      html.matchAll(/(?:href|src)="(\/Sky-Auto-Player\/[^"#?]+)"/g),
      (match) => match[1],
    );

    for (const url of new Set(localUrls)) {
      const response = await request.get(hostOrigin + url);
      expect(response.status(), url).toBe(200);
    }
  });
});

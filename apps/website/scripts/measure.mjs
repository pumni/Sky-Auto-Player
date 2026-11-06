import { chromium } from '@playwright/test';
import { spawn } from 'child_process';

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();

  // Start server or assume it's running? The task before ran build.
  // Let's just use a local file or start a server.
  // Actually, we can run `bun run preview` in background, or just use the local file server.
  // We can use the preview server URL http://localhost:4321/Sky-Auto-Player/

  // Since we need a server, we can spawn one
  const server = spawn('bun', ['run', 'preview'], { stdio: 'ignore' });

  // wait a bit for server
  await new Promise((r) => setTimeout(r, 2000));

  try {
    const widths = [320, 390, 768, 1024, 1280, 1440];
    for (const w of widths) {
      await page.setViewportSize({ width: w, height: 900 });
      await page.goto('http://localhost:4321/Sky-Auto-Player/');
      await page.waitForLoadState('networkidle');

      const metrics = await page.evaluate(() => {
        return {
          windowInnerWidth: window.innerWidth,
          bodyScrollWidth: document.body.scrollWidth,
          docScrollWidth: document.documentElement.scrollWidth,
        };
      });
      console.log(`Viewport ${w}px:`, metrics);

      if (metrics.docScrollWidth > metrics.windowInnerWidth) {
        // find top offending elements
        const offenders = await page.evaluate(() => {
          const w = window.innerWidth;
          const all = Array.from(document.querySelectorAll('*'));
          return all
            .filter((el) => {
              const rect = el.getBoundingClientRect();
              return rect.right > w;
            })
            .map((el) => ({
              tag: el.tagName,
              className: el.className,
              right: el.getBoundingClientRect().right,
            }))
            .sort((a, b) => b.right - a.right)
            .slice(0, 3);
        });
        console.log(`Offending elements at ${w}px:`, offenders);
      }
    }

    // Step 8: Measure hero header-to-kicker gap, padding, etc. at 1440px
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('http://localhost:4321/Sky-Auto-Player/');
    await page.waitForLoadState('networkidle');

    const layoutMetrics = await page.evaluate(() => {
      const hero = document.querySelector('.hero');
      const header = document.querySelector('header');
      const kicker = document.querySelector('.kicker, .hero__kicker');
      const productImage = document.querySelector('.product-view img, .product-showcase img');
      const teaserRows = document.querySelectorAll('.teaser-row, .event-row, .hero__event');

      let headerToKickerGap = -1;
      if (header && kicker) {
        headerToKickerGap =
          kicker.getBoundingClientRect().top - header.getBoundingClientRect().bottom;
      }

      return {
        heroPadding: hero ? window.getComputedStyle(hero).padding : 'N/A',
        headerToKickerGap,
        productImageBounds: productImage ? productImage.getBoundingClientRect() : 'N/A',
        teaserRowCount: teaserRows.length,
      };
    });
    console.log('Layout metrics at 1440px:', layoutMetrics);
  } finally {
    await browser.close();
    server.kill();
  }
})();

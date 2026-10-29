import { expect, test } from '@playwright/test';

const viewports = [
  { width: 1366, height: 768 },
  { width: 995, height: 706 },
  { width: 800, height: 560 },
  { width: 640, height: 448 },
] as const;

const fixtures = [
  { name: 'none', length: 0 },
  { name: 'short', length: 57 },
  { name: 'medium', length: 1_024 },
  { name: 'long', length: 4_096 },
  { name: 'maximum', length: 16_384 },
] as const;

const finalNoteLine = 'FINAL RELEASE NOTE LINE — Hoàn tất cập nhật.';

test('update modal adapts to release-note length and viewport geometry', async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);

  for (const viewport of viewports) {
    for (const fixture of fixtures) {
      await page.setViewportSize(viewport);
      await page.goto(`/?mockReleaseNotes=${fixture.name}`);
      await page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ }).click();

      const dialog = page.getByRole('dialog', { name: 'Software update' });
      await expect(dialog).toBeVisible();
      const updateButton = dialog.getByRole('button', { name: 'Update and restart' });
      const laterButton = dialog.getByRole('button', { name: 'Later' });
      const skipButton = dialog.getByRole('button', { name: 'Skip this version' });
      const closeButton = dialog.getByRole('button', { name: 'Close update' });
      const notesRegion = dialog.getByRole('region', { name: 'What’s new' });

      await expect(updateButton).toBeVisible();
      await expect(laterButton).toBeVisible();
      await expect(skipButton).toBeVisible();
      await expect(closeButton).toBeVisible();
      await expect(dialog.getByRole('status')).toHaveCount(1);

      const geometry = await dialog.evaluate((element) => {
        const rect = (node: Element) => {
          const { x, y, width, height } = node.getBoundingClientRect();
          return { x, y, width, height };
        };
        const header = element.querySelector<HTMLElement>('.dialog-heading');
        const footer = element.querySelector<HTMLElement>('.update-actions');
        const notesRegion = element.querySelector<HTMLElement>('.update-notes-region');
        const notes = element.querySelector<HTMLElement>('.update-notes');
        const backdrop = document.querySelector<HTMLElement>('.modal-backdrop');
        if (!header || !footer || !backdrop) return null;
        return {
          viewport: { width: window.innerWidth, height: window.innerHeight },
          dialog: rect(element),
          header: rect(header),
          footer: rect(footer),
          notesRegion: notesRegion
            ? {
                rect: rect(notesRegion),
                overflowY: getComputedStyle(notesRegion).overflowY,
                clientWidth: notesRegion.clientWidth,
                scrollWidth: notesRegion.scrollWidth,
                clientHeight: notesRegion.clientHeight,
                scrollHeight: notesRegion.scrollHeight,
              }
            : null,
          notesLength: notes?.textContent?.length ?? 0,
          activeScrollports: [...element.querySelectorAll<HTMLElement>('*')]
            .filter((node) => {
              const style = getComputedStyle(node);
              return (
                ['auto', 'scroll'].includes(style.overflowY) &&
                node.scrollHeight > node.clientHeight + 1
              );
            })
            .map((node) => node.className),
          backdrop: {
            clientHeight: backdrop.clientHeight,
            scrollHeight: backdrop.scrollHeight,
          },
          document: {
            clientWidth: document.documentElement.clientWidth,
            scrollWidth: document.documentElement.scrollWidth,
          },
        };
      });

      expect(geometry).not.toBeNull();
      expect(geometry?.notesLength).toBe(fixture.length);
      if (geometry?.notesRegion) {
        expect(geometry.notesRegion.scrollWidth).toBeLessThanOrEqual(
          geometry.notesRegion.clientWidth + 1,
        );
      }
      const notesOverflow =
        (geometry?.notesRegion?.scrollHeight ?? 0) > (geometry?.notesRegion?.clientHeight ?? 0) + 1;
      expect(geometry?.activeScrollports).toEqual(notesOverflow ? ['update-notes-region'] : []);
      expect(geometry?.backdrop.scrollHeight).toBeLessThanOrEqual(
        (geometry?.backdrop.clientHeight ?? 0) + 1,
      );
      expect(geometry?.document.scrollWidth).toBeLessThanOrEqual(
        geometry?.document.clientWidth ?? 0,
      );

      const dialogBox = await dialog.boundingBox();
      expect(dialogBox).not.toBeNull();
      if (dialogBox) {
        expect(dialogBox.x).toBeGreaterThanOrEqual(16);
        expect(dialogBox.y).toBeGreaterThanOrEqual(16);
        expect(dialogBox.x + dialogBox.width).toBeLessThanOrEqual(viewport.width - 16);
        expect(dialogBox.y + dialogBox.height).toBeLessThanOrEqual(viewport.height - 16);
        if (fixture.name === 'none' || fixture.name === 'short') {
          expect(dialogBox.height).toBeLessThan(400);
        }
      }

      for (const button of [updateButton, laterButton, skipButton, closeButton]) {
        const box = await button.boundingBox();
        expect(box).not.toBeNull();
        if (box && dialogBox) {
          expect(box.height).toBeGreaterThanOrEqual(32);
          expect(box.x).toBeGreaterThanOrEqual(dialogBox.x);
          expect(box.y).toBeGreaterThanOrEqual(dialogBox.y);
          expect(box.x + box.width).toBeLessThanOrEqual(dialogBox.x + dialogBox.width);
          expect(box.y + box.height).toBeLessThanOrEqual(dialogBox.y + dialogBox.height);
          expect(
            await button.evaluate(
              (node, point) => {
                const hit = document.elementFromPoint(point.x, point.y);
                return hit === node || (hit instanceof Node && node.contains(hit));
              },
              { x: box.x + box.width / 2, y: box.y + box.height / 2 },
            ),
          ).toBe(true);
        }
      }

      if (fixture.length === 0) {
        await expect(notesRegion).toHaveCount(0);
      } else {
        await expect(notesRegion).toBeVisible();
        await expect(notesRegion).toHaveAttribute('tabindex', '0');
        const notes = notesRegion.locator('.update-notes');
        await expect(dialog.locator('[aria-live]')).toHaveCount(1);
        await expect(dialog.getByRole('status')).not.toContainText('Unbroken text:');
        if (fixture.name === 'short') {
          await expect(notes).toHaveText(
            'Bản cập nhật cải thiện độ ổn định và trải nghiệm sử dụng.',
          );
        } else {
          await expect(notes).toContainText('Cải thiện độ ổn định');
          await expect(notes).toContainText('Unbroken text:');
          await expect(notes).toContainText('<img src=x onerror=alert(1)>');
          await expect(notes.locator('img, script, a')).toHaveCount(0);
        }

        if (fixture.name === 'long' || fixture.name === 'maximum') {
          expect(geometry?.notesRegion?.clientHeight).toBeGreaterThan(120);
          expect(geometry?.notesRegion?.scrollHeight).toBeGreaterThan(
            geometry?.notesRegion?.clientHeight ?? 0,
          );
        } else if (fixture.name === 'none' || fixture.name === 'short') {
          expect(geometry?.notesRegion?.scrollHeight).toBeLessThanOrEqual(
            geometry?.notesRegion?.clientHeight ?? 0,
          );
        }

        if (fixture.name === 'maximum') {
          if (viewport.width === 995 || viewport.width === 640) {
            console.log(
              `UPDATE_MODAL_AFTER_GEOMETRY ${JSON.stringify({
                viewport,
                fixture: fixture.name,
                dialog: geometry?.dialog,
                notesRegion: geometry?.notesRegion,
                footer: geometry?.footer,
              })}`,
            );
          }
          if (viewport.width === 995 || viewport.width === 640) {
            await page.screenshot({
              path: testInfo.outputPath(
                `update-modal-after-${viewport.width}x${viewport.height}.png`,
              ),
            });
          }

          await notesRegion.evaluate((element) => {
            element.scrollTop = 0;
          });
          const firstLine = await notes.evaluate((element) => {
            const textNode = element.firstChild;
            if (!textNode || textNode.nodeType !== Node.TEXT_NODE) return null;
            const range = document.createRange();
            range.setStart(textNode, 0);
            range.setEnd(textNode, Math.min(32, textNode.textContent?.length ?? 0));
            const { top, bottom } = range.getBoundingClientRect();
            const port = element.closest<HTMLElement>('.update-notes-region');
            const portRect = port?.getBoundingClientRect();
            return portRect
              ? { top, bottom, portTop: portRect.top, portBottom: portRect.bottom }
              : null;
          });
          expect(firstLine).not.toBeNull();
          expect(firstLine?.top).toBeGreaterThanOrEqual(firstLine?.portTop ?? 0);
          expect(firstLine?.bottom).toBeLessThanOrEqual(firstLine?.portBottom ?? 0);

          await notesRegion.focus();
          await page.keyboard.press('End');
          await expect
            .poll(() => notesRegion.evaluate((element) => element.scrollTop))
            .toBeGreaterThan(0);
          const lastLine = await notes.evaluate((element, marker) => {
            const textNode = element.firstChild;
            if (!textNode || textNode.nodeType !== Node.TEXT_NODE) return null;
            const range = document.createRange();
            const start = Math.max(0, (textNode.textContent?.length ?? 0) - marker.length);
            range.setStart(textNode, start);
            range.setEnd(textNode, textNode.textContent?.length ?? start);
            const { top, bottom } = range.getBoundingClientRect();
            const port = element.closest<HTMLElement>('.update-notes-region');
            const portRect = port?.getBoundingClientRect();
            return portRect
              ? { top, bottom, portTop: portRect.top, portBottom: portRect.bottom }
              : null;
          }, finalNoteLine);
          expect(lastLine).not.toBeNull();
          expect(lastLine?.top).toBeGreaterThanOrEqual(lastLine?.portTop ?? 0);
          expect(lastLine?.bottom).toBeLessThanOrEqual(lastLine?.portBottom ?? 0);
        }
      }
    }
  }
});

test('update dialog remains usable at 125% and 150% browser zoom', async ({ page }) => {
  for (const zoom of [1.25, 1.5]) {
    await page.setViewportSize({ width: 995, height: 706 });
    await page.goto('/?mockReleaseNotes=maximum');
    await page.evaluate((scale) => {
      document.documentElement.style.zoom = String(scale);
    }, zoom);
    await page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ }).click();

    const dialog = page.getByRole('dialog', { name: 'Software update' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Update and restart' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Later' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Skip this version' })).toBeVisible();
    const overflow = await page.evaluate(() => ({
      viewportWidth: document.documentElement.clientWidth,
      documentWidth: document.documentElement.scrollWidth,
      viewportHeight: document.documentElement.clientHeight,
      documentHeight: document.documentElement.scrollHeight,
    }));
    expect(overflow.documentWidth).toBeLessThanOrEqual(overflow.viewportWidth);
    expect(overflow.documentHeight).toBeLessThanOrEqual(overflow.viewportHeight);
  }
});

test('long update errors scroll without hiding retry or dismissal controls', async ({ page }) => {
  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/?mockUpdateState=error-long');
  await page.getByRole('button', { name: 'Open settings' }).click();
  await page.getByRole('button', { name: 'Updates', exact: true }).click();
  const checkButton = page.getByRole('button', { name: 'Check for updates', exact: true });
  await checkButton.click();

  const dialog = page.getByRole('dialog', { name: 'Software update' });
  const errorText = dialog.locator('.update-error-region');
  const checkAgain = dialog.getByRole('button', { name: 'Check again' });
  const close = dialog.getByRole('button', { name: 'Close', exact: true });
  await expect(dialog.getByRole('heading', { name: 'Update check failed' })).toBeVisible();
  await expect(errorText).toBeVisible();
  await expect(errorText).toHaveText(/Cải thiện độ ổn định/);
  await expect
    .poll(() => errorText.evaluate((element) => element.textContent?.length ?? 0))
    .toBe(4_096);
  await expect(checkAgain).toBeVisible();
  await expect(close).toBeVisible();
  await expect(dialog.locator('[aria-live]')).toHaveCount(1);
  await expect(dialog.getByRole('status')).not.toContainText('Cải thiện độ ổn định');

  const geometry = await dialog.evaluate((element) => {
    const error = element.querySelector<HTMLElement>('.update-error-region');
    const backdrop = document.querySelector<HTMLElement>('.modal-backdrop');
    if (!error || !backdrop) return null;
    return {
      errorClientHeight: error.clientHeight,
      errorClientWidth: error.clientWidth,
      errorScrollHeight: error.scrollHeight,
      errorScrollWidth: error.scrollWidth,
      activeScrollports: [...element.querySelectorAll<HTMLElement>('*')]
        .filter((node) => {
          const style = getComputedStyle(node);
          return (
            ['auto', 'scroll'].includes(style.overflowY) &&
            node.scrollHeight > node.clientHeight + 1
          );
        })
        .map((node) => node.className),
      backdropClientHeight: backdrop.clientHeight,
      backdropScrollHeight: backdrop.scrollHeight,
    };
  });
  expect(geometry?.errorScrollHeight).toBeGreaterThan(geometry?.errorClientHeight ?? 0);
  expect(geometry?.errorScrollWidth).toBeLessThanOrEqual((geometry?.errorClientWidth ?? 0) + 1);
  expect(geometry?.activeScrollports).toEqual(['inline-error update-error-region']);
  expect(geometry?.backdropScrollHeight).toBeLessThanOrEqual(
    (geometry?.backdropClientHeight ?? 0) + 1,
  );
});

test('update dialog traps keyboard focus and restores it after Escape and outside dismissal', async ({
  page,
}) => {
  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/');
  const trigger = page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ });
  await trigger.focus();
  await page.keyboard.press('Enter');

  const dialog = page.getByRole('dialog', { name: 'Software update' });
  const close = dialog.getByRole('button', { name: 'Close update' });
  const skip = dialog.getByRole('button', { name: 'Skip this version' });
  await expect(dialog).toBeVisible();
  await skip.focus();
  await page.keyboard.press('Tab');
  await expect(close).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(skip).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();

  await trigger.press('Enter');
  await expect(dialog).toBeVisible();
  await page.mouse.click(8, 8);
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
});

test('busy update cannot close from Escape, outside click, or the header button', async ({
  page,
}) => {
  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/?mockReleaseNotes=short');
  await page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Software update' });
  await dialog.getByRole('button', { name: 'Update and restart' }).click();
  await expect(dialog.getByRole('heading', { name: 'Installing update…' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Close update' })).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(dialog).toBeVisible();
  await page.mouse.click(8, 8);
  await expect(dialog).toBeVisible();
});

test('update notes and actions remain visible with forced colors and reduced motion', async ({
  page,
}) => {
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 640, height: 448 });
  await page.goto('/?mockReleaseNotes=maximum');
  await page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ }).click();

  const dialog = page.getByRole('dialog', { name: 'Software update' });
  await expect(dialog.getByRole('region', { name: 'What’s new' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Update and restart' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Later' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Skip this version' })).toBeVisible();
});

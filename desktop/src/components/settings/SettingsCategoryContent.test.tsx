import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { SettingsCategoryContent, SETTINGS_CATEGORIES } from './SettingsCategoryContent';
import type { SettingsCategory } from './SettingsCategoryContent';

afterEach(cleanup);

async function initializedStore() {
  const store = createDesktopStore(createMockBridge());
  await act(async () => store.getState().initialize());
  const bootstrap = store.getState().bootstrap;
  const settings = store.getState().settings;
  if (!bootstrap || !settings) throw new Error('settings store should be initialized');
  return { store, bootstrap, settings };
}

describe('SettingsCategoryContent', () => {
  it('keeps the existing settings category order', () => {
    expect(SETTINGS_CATEGORIES.map(({ id }) => id)).toEqual([
      'playback',
      'appearance',
      'diagnostics',
      'updates',
      'advanced',
      'about',
    ]);
  });

  it.each([
    ['playback', 'Playback'],
    ['appearance', 'Appearance'],
    ['diagnostics', 'Telemetry and diagnostics'],
    ['updates', 'Updates'],
    ['advanced', 'Advanced'],
    ['about', 'About Sky Auto Player'],
  ] as const)('renders the %s section with its accessible name', async (category, name) => {
    const { store, bootstrap, settings } = await initializedStore();

    render(
      <SettingsCategoryContent
        category={category as SettingsCategory}
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );

    expect(screen.getByRole('region', { name })).toBeInTheDocument();
  });

  it('keeps the existing six persisted palette IDs and labels', async () => {
    const { store, bootstrap, settings } = await initializedStore();
    render(
      <SettingsCategoryContent
        category="appearance"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );

    const palette = screen.getByLabelText('Color palette');
    expect(
      Array.from((palette as HTMLSelectElement).options).map((option) => [
        option.value,
        option.textContent,
      ]),
    ).toEqual([
      ['graphite', 'Graphite (Default)'],
      ['aurora', 'Aurora'],
      ['minimalist', 'Minimalist'],
      ['slate', 'Slate'],
      ['cyberpunk', 'Cyberpunk'],
      ['classic', 'Classic'],
    ]);

    await act(async () => {
      (palette as HTMLSelectElement).value = 'slate';
      palette.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await waitFor(() => expect(store.getState().settings?.palette).toBe('slate'));
  });
});

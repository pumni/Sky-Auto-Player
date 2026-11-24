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
      'donate',
      'about',
    ]);
  });

  it.each([
    ['playback', 'Playback'],
    ['appearance', 'Appearance'],
    ['diagnostics', 'Telemetry and diagnostics'],
    ['updates', 'Updates'],
    ['donate', 'Support Sky Auto Player'],
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

    const palette = screen.getByRole('combobox', { name: 'Dark palette' });
    expect(palette).toHaveAttribute('data-value', 'graphite');
    await act(async () => palette.click());

    const options = screen.getAllByRole('option');
    expect(
      options.map((option) => [
        option.getAttribute('data-value'),
        option.querySelector('.app-select-option-label')?.textContent,
      ]),
    ).toEqual([
      ['graphite', 'Graphite (Default)'],
      ['aurora', 'Aurora'],
      ['minimalist', 'Minimalist'],
      ['slate', 'Slate'],
      ['cyberpunk', 'Cyberpunk'],
      ['classic', 'Classic'],
    ]);

    await act(async () => screen.getByRole('option', { name: 'Slate' }).click());
    await waitFor(() => expect(store.getState().settings?.palette).toBe('slate'));
    expect(store.getState().settings?.appearance_mode).toBe('system');
  });

  it('keeps theme choice independent from the saved dark palette', async () => {
    const { store, bootstrap, settings } = await initializedStore();
    render(
      <SettingsCategoryContent
        category="appearance"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );

    const theme = screen.getByRole('combobox', { name: 'Theme' });
    expect(theme).toHaveAttribute('data-value', 'system');
    await act(async () => theme.click());

    const themeOptions = screen.getAllByRole('option');
    expect(
      themeOptions.map((option) => [
        option.getAttribute('data-value'),
        option.querySelector('.app-select-option-label')?.textContent,
      ]),
    ).toEqual([
      ['system', 'Follow Windows (recommended)'],
      ['light', 'Light'],
      ['dark', 'Dark'],
    ]);

    await act(async () => screen.getByRole('option', { name: 'Light' }).click());
    await waitFor(() => expect(store.getState().settings?.appearance_mode).toBe('light'));
    expect(store.getState().settings?.palette).toBe('graphite');

    const palette = screen.getByRole('combobox', { name: 'Dark palette' });
    await act(async () => palette.click());
    await act(async () => screen.getByRole('option', { name: 'Slate' }).click());
    await waitFor(() => expect(store.getState().settings?.palette).toBe('slate'));
    expect(store.getState().settings?.appearance_mode).toBe('light');
  });

  it('keeps exact accessible checkbox labels in Diagnostics and Updates', async () => {
    const { store, bootstrap, settings } = await initializedStore();
    const { rerender } = render(
      <SettingsCategoryContent
        category="diagnostics"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );
    expect(screen.getByRole('checkbox', { name: 'Allow anonymous telemetry' })).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: 'Verbose playback HUD' })).toBeInTheDocument();

    rerender(
      <SettingsCategoryContent
        category="updates"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );
    expect(
      screen.getByRole('checkbox', { name: 'Check for updates automatically' }),
    ).toBeInTheDocument();
  });
});

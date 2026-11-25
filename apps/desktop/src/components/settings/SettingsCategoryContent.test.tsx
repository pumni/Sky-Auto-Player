import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import type { SettingsCategory } from './SettingsCategoryContent';
import { SETTINGS_CATEGORIES, SettingsCategoryContent } from './SettingsCategoryContent';

afterEach(cleanup);

async function initializedStore() {
  const bridge = createMockBridge();
  const store = createDesktopStore(bridge);
  await act(async () => store.getState().initialize());
  const bootstrap = store.getState().bootstrap;
  const settings = store.getState().settings;
  if (!bootstrap || !settings) throw new Error('settings store should be initialized');
  return { bridge, store, bootstrap, settings };
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
    ['diagnostics', 'Diagnostics'],
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
    expect(palette).toHaveTextContent('Graphite (Default)');
    await act(async () => palette.click());

    const options = screen.getAllByRole('option');
    expect(options.map((option) => option.textContent?.trim())).toEqual([
      'Graphite (Default)',
      'Aurora',
      'Minimalist',
      'Slate',
      'Cyberpunk',
      'Classic',
    ]);

    await act(async () => screen.getByRole('option', { name: 'Slate' }).click());
    await waitFor(() => expect(store.getState().settings?.palette).toBe('slate'));
    expect(store.getState().settings?.appearance_mode).toBe('system');
  });

  it('previews the saved dark palette and labels the effective color mode', async () => {
    const { store, bootstrap, settings } = await initializedStore();
    const { rerender, container } = render(
      <SettingsCategoryContent
        category="appearance"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );

    const darkPreview = screen.getByRole('img', {
      name: 'Dark appearance preview using saved palette Graphite (Default)',
    });
    const appearanceSummary = container.querySelector('[data-slot="appearance-active-summary"]');
    expect(darkPreview).toHaveAttribute('data-effective-mode', 'dark');
    expect(appearanceSummary).toHaveTextContent(
      'Active: Dark. Saved dark palette: Graphite (Default).',
    );

    rerender(
      <SettingsCategoryContent
        category="appearance"
        bootstrap={bootstrap}
        settings={{ ...settings, appearance_mode: 'light' }}
        useStore={store}
      />,
    );
    expect(
      screen.getByRole('img', {
        name: 'Light appearance preview using saved palette Graphite (Default)',
      }),
    ).toHaveAttribute('data-effective-mode', 'light');
    expect(appearanceSummary).toHaveTextContent(
      'Active: Light. Saved dark palette: Graphite (Default).',
    );
  });

  it('uses a radio group for Theme choices and keeps Dark palette independent', async () => {
    const { store, bootstrap, settings } = await initializedStore();
    const { rerender } = render(
      <SettingsCategoryContent
        category="appearance"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );

    const themeGroup = screen.getByRole('radiogroup', { name: 'Theme' });
    expect(themeGroup).toBeInTheDocument();
    const followWindows = screen.getByRole('radio', { name: /Follow Windows \(recommended\)/ });
    const light = screen.getByRole('radio', { name: 'Light' });
    const dark = screen.getByRole('radio', { name: 'Dark' });
    expect(followWindows).toHaveAttribute('aria-checked', 'true');
    expect(light).toHaveAttribute('aria-checked', 'false');
    expect(dark).toHaveAttribute('aria-checked', 'false');

    await act(async () => {
      fireEvent.click(light);
    });
    await waitFor(() => expect(store.getState().settings?.appearance_mode).toBe('light'));
    expect(store.getState().settings?.palette).toBe('graphite');

    rerender(
      <SettingsCategoryContent
        category="appearance"
        bootstrap={bootstrap}
        settings={store.getState().settings ?? settings}
        useStore={store}
      />,
    );
    const palette = screen.getByRole('combobox', { name: 'Dark palette' });
    expect(palette).toBeEnabled();
    await act(async () => palette.click());
    await act(async () => screen.getByRole('option', { name: 'Slate' }).click());
    await waitFor(() => expect(store.getState().settings?.palette).toBe('slate'));
    expect(store.getState().settings?.appearance_mode).toBe('light');
  });

  it('uses truthful switch controls in Diagnostics and persists toggles', async () => {
    const { store, bootstrap, settings } = await initializedStore();
    const { rerender } = render(
      <SettingsCategoryContent
        category="diagnostics"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
      />,
    );

    const telemetrySwitch = screen.getByRole('switch', { name: 'Playback telemetry preference' });
    const verboseSwitch = screen.getByRole('switch', { name: 'Verbose diagnostics preference' });
    expect(telemetrySwitch).toHaveAccessibleDescription('Stores your telemetry preference.');
    expect(verboseSwitch).toHaveAccessibleDescription(
      'Stores your verbose diagnostics preference.',
    );
    expect(
      screen.getByText(
        'These preferences are saved, but this build does not establish a data-sharing or additional display effect.',
      ),
    ).toBeInTheDocument();

    await act(async () => {
      telemetrySwitch.click();
    });
    await waitFor(() => expect(store.getState().settings?.telemetry_enabled).toBe(false));

    rerender(
      <SettingsCategoryContent
        category="diagnostics"
        bootstrap={bootstrap}
        settings={store.getState().settings ?? settings}
        useStore={store}
      />,
    );
    await act(async () => {
      screen.getByRole('switch', { name: 'Verbose diagnostics preference' }).click();
    });
    await waitFor(() => expect(store.getState().settings?.verbose_hud).toBe(true));
  });

  it('renders truthful Updates status states and opens UpdateDialog without duplicate checks', async () => {
    const { bridge, store, bootstrap, settings } = await initializedStore();
    const checkSpy = vi.spyOn(bridge, 'checkForUpdate');
    const onUpdateDialogOpen = vi.fn();

    // 1. Available state (default mock after initialize) -> View update opens dialog without check
    const { rerender } = render(
      <SettingsCategoryContent
        category="updates"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
        onUpdateDialogOpen={onUpdateDialogOpen}
      />,
    );
    expect(
      screen.getByRole('switch', { name: 'Check for updates automatically' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: 'Release channel' })).toHaveTextContent('Stable');
    expect(screen.getByText(/Update available:/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'View update' }));
    expect(onUpdateDialogOpen).toHaveBeenCalledTimes(1);
    expect(store.getState().update.dialogOpen).toBe(true);
    expect(checkSpy).not.toHaveBeenCalled();

    // 2. Idle state -> Check for updates triggers manual check once
    act(() => {
      store.setState((prev) => ({
        update: { ...prev.update, state: 'idle', availableVersion: null, dialogOpen: false },
      }));
    });
    rerender(
      <SettingsCategoryContent
        category="updates"
        bootstrap={bootstrap}
        settings={settings}
        useStore={store}
        onUpdateDialogOpen={onUpdateDialogOpen}
      />,
    );
    expect(screen.queryByText("You're up to date")).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Check for updates' })).toBeEnabled();

    // 3. Current state -> You're up to date
    act(() => {
      store.setState((prev) => ({
        update: { ...prev.update, state: 'current', availableVersion: null },
      }));
    });
    expect(screen.getByText("You're up to date")).toBeInTheDocument();

    // 4. Checking state -> disabled Checking for updates…
    act(() => {
      store.setState((prev) => ({
        update: { ...prev.update, state: 'checking', checkRequestPending: true },
      }));
    });
    expect(screen.getByRole('button', { name: 'Checking for updates…' })).toBeDisabled();

    // 5. Downloading state -> truthful status + View update
    act(() => {
      store.setState((prev) => ({
        update: { ...prev.update, state: 'downloading', checkRequestPending: false },
      }));
    });
    expect(screen.getByText('Downloading update…')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'View update' })).toBeEnabled();

    // 6. Error state with retryAction: 'check' -> Check again + View details
    act(() => {
      store.setState((prev) => ({
        update: {
          ...prev.update,
          state: 'error',
          errorCode: 'check_failed',
          retryAction: 'check',
        },
      }));
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Could not check for updates.');
    expect(screen.getByRole('button', { name: 'Check again' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'View details' })).toBeEnabled();

    // 7. Error state with retryAction: 'none' -> View details only (no guessing)
    act(() => {
      store.setState((prev) => ({
        update: {
          ...prev.update,
          state: 'error',
          errorCode: 'unknown',
          retryAction: 'none',
        },
      }));
    });
    expect(screen.queryByRole('button', { name: 'Check again' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'View details' })).toBeEnabled();
  });
});

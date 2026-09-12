import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { SettingsPanel } from './SettingsPanel';

describe('SettingsPanel playback timing', () => {
  afterEach(() => cleanup());

  it('labels base hold and shows the independent cutoff and recommendation source', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const bootstrap = store.getState().bootstrap;
    if (!bootstrap) throw new Error('bootstrap should be available');
    act(() => store.getState().setSettingsOpen(true));

    render(<SettingsPanel bootstrap={bootstrap} useStore={store} />);

    expect(screen.getByLabelText('Base Hold')).toBeInTheDocument();
    expect(screen.getByText('Late Down tolerance').parentElement).toHaveTextContent('2000 µs');
    expect(screen.getByText(/Recommended sender margin: 2300 µs · Source:/)).toHaveTextContent(
      'Default fallback (no valid calibration cache)',
    );
  });
});

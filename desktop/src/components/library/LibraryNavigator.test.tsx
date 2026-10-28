import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { LibraryNavigator } from './LibraryNavigator';

describe('LibraryNavigator built-in source', () => {
  afterEach(cleanup);

  it('shows the native built-in count and disables an unavailable source', async () => {
    const store = createDesktopStore(
      createMockBridge({ builtinAvailable: false, builtinSongIds: [] }),
    );
    await act(async () => store.getState().initialize());
    render(
      <LibraryNavigator collapsed={false} onToggleCollapsed={() => undefined} useStore={store} />,
    );

    const builtin = screen.getByRole('button', { name: /Built-in Songs/ });
    expect(builtin).toBeDisabled();
    expect(builtin).toHaveTextContent('0');
    expect(screen.getByRole('button', { name: /All Songs/ })).toBeEnabled();
  });
});

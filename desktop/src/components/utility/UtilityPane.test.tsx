import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { UtilityPane } from './UtilityPane';

type DesktopStore = ReturnType<typeof createDesktopStore>;

function Fixture({ store }: { store: DesktopStore }) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button ref={triggerRef} type="button">
        Utility trigger
      </button>
      <UtilityPane presentationMode="docked" utilityTriggerRef={triggerRef} useStore={store} />
    </>
  );
}

function createOpenStore() {
  const store = createDesktopStore(createMockBridge());
  act(() => store.getState().openUtility('details'));
  return store;
}

afterEach(cleanup);

describe('UtilityPane', () => {
  it('navigates the utility views through React Aria tabs and loads Diagnostics lazily', async () => {
    const store = createOpenStore();
    render(<Fixture store={store} />);

    expect(screen.getByRole('region', { name: 'Utility: Song Details' })).toBeInTheDocument();
    const detailsTab = screen.getByRole('tab', { name: 'Details' });
    expect(detailsTab).toHaveAttribute('aria-selected', 'true');

    fireEvent.click(screen.getByRole('tab', { name: 'Runtime' }));

    expect(store.getState().utility.activeView).toBe('diagnostics');
    expect(await screen.findByRole('tab', { name: 'Performance' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Utility: Diagnostics' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Runtime' })).toHaveAttribute('aria-selected', 'true');
  });

  it('keeps keyboard tab selection and close focus restoration intact', async () => {
    const store = createOpenStore();
    render(<Fixture store={store} />);

    const detailsTab = screen.getByRole('tab', { name: 'Details' });
    detailsTab.focus();
    fireEvent.keyDown(detailsTab, { key: 'ArrowRight' });

    await waitFor(() => expect(store.getState().utility.activeView).toBe('diagnostics'));
    const selectedRuntimeTab = screen.getByRole('tab', { name: 'Runtime' });
    expect(selectedRuntimeTab).toHaveAttribute('aria-selected', 'true');
    expect(selectedRuntimeTab).toHaveFocus();

    fireEvent.keyDown(selectedRuntimeTab, { key: 'ArrowLeft' });

    await waitFor(() => expect(store.getState().utility.activeView).toBe('details'));
    expect(screen.getByRole('tab', { name: 'Details' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'Details' })).toHaveFocus();

    const region = screen.getByRole('region', { name: 'Utility: Song Details' });
    fireEvent.keyDown(region, { key: 'Escape' });

    await waitFor(() => expect(store.getState().utility.open).toBe(false));
    expect(screen.queryByRole('region', { name: 'Utility: Song Details' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Utility trigger' })).toHaveFocus();
  });
});

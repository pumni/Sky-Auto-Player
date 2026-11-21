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

  it('uses Collapse Inspector in docked mode and renders a symmetric 48px icon rail when collapsed', () => {
    const store = createOpenStore();
    let collapsed = false;
    function CollapsibleFixture({ isCollapsed }: { isCollapsed: boolean }) {
      const triggerRef = useRef<HTMLButtonElement>(null);
      return (
        <>
          <button ref={triggerRef} type="button">
            Utility trigger
          </button>
          <UtilityPane
            presentationMode="docked"
            collapsed={isCollapsed}
            onToggleCollapsed={() => {
              collapsed = !collapsed;
            }}
            onExpand={() => {
              collapsed = false;
            }}
            utilityTriggerRef={triggerRef}
            useStore={store}
          />
        </>
      );
    }

    const { rerender } = render(<CollapsibleFixture isCollapsed={false} />);
    const collapseButton = screen.getByRole('button', { name: 'Collapse Inspector' });
    expect(screen.queryByRole('button', { name: 'Close utility' })).toBeNull();
    fireEvent.click(collapseButton);
    expect(collapsed).toBe(true);

    rerender(<CollapsibleFixture isCollapsed={true} />);
    expect(screen.getByRole('button', { name: 'Expand Inspector' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Song Details' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    const runtimeTile = screen.getByRole('button', { name: 'Runtime' });
    expect(runtimeTile).toHaveAttribute('aria-pressed', 'false');

    fireEvent.click(runtimeTile);
    expect(collapsed).toBe(false);
    expect(store.getState().utility.activeView).toBe('diagnostics');
  });

  it('keeps Close utility button in overlay mode', async () => {
    const store = createOpenStore();
    function OverlayFixture() {
      const triggerRef = useRef<HTMLButtonElement>(null);
      return (
        <>
          <button ref={triggerRef} type="button">
            Utility trigger
          </button>
          <UtilityPane
            presentationMode="overlay"
            collapsed={true}
            onToggleCollapsed={() => {}}
            utilityTriggerRef={triggerRef}
            useStore={store}
          />
        </>
      );
    }

    render(<OverlayFixture />);
    expect(screen.queryByRole('button', { name: 'Collapse Inspector' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Close utility' }));
    await waitFor(() => expect(store.getState().utility.open).toBe(false));
  });
});

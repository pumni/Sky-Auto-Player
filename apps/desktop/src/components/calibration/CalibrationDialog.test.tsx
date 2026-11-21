import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { CalibrationDialog } from './CalibrationDialog';

afterEach(cleanup);

describe('CalibrationDialog', () => {
  it('renders compact idle, running, and semantic terminal states', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    act(() => store.getState().setCalibrationOpen(true));

    render(<CalibrationDialog useStore={store} />);
    const dialog = screen.getByRole('dialog', { name: 'Timing calibration' });
    expect(dialog).toHaveClass('calibration-dialog');
    expect(dialog).not.toHaveClass('settings-dialog');
    expect(dialog.querySelector('button[aria-label="Close calibration"] svg')).toHaveClass(
      'lucide-x',
    );

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Start quick calibration' }));
    });
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: 'Calibration complete' })).toBeInTheDocument(),
    );
    expect(dialog.querySelector('.calibration-result-card--success')).not.toBeNull();
  });

  it('uses danger and warning semantics for failed and cancelled states', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    act(() => {
      store.setState((state) => ({
        calibration: {
          ...state.calibration,
          open: true,
          state: 'failed',
          error: 'Could not initialize timer.',
        },
      }));
    });

    const { rerender } = render(<CalibrationDialog useStore={store} />);
    expect(document.querySelector('.calibration-result-card--danger')).not.toBeNull();
    expect(document.querySelector('.calibration-result-icon--danger')).not.toBeNull();
    expect(screen.getByText('Could not initialize timer.')).toBeInTheDocument();

    act(() => {
      store.setState((state) => ({
        calibration: {
          ...state.calibration,
          open: true,
          state: 'cancelled',
          message: 'Cancelled by user.',
        },
      }));
    });
    rerender(<CalibrationDialog useStore={store} />);
    expect(document.querySelector('.calibration-result-card--warning')).not.toBeNull();
    expect(document.querySelector('.calibration-result-icon--warning')).not.toBeNull();
    expect(screen.getByText('Cancelled by user.')).toBeInTheDocument();
  });
});

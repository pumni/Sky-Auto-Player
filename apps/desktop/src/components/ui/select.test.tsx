import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './select';

afterEach(cleanup);

function ExampleSelect({ onValueChange }: { onValueChange: (value: string | null) => void }) {
  return (
    <div>
      <label htmlFor="playback-mode">Playback mode</label>
      <Select defaultValue="normal" onValueChange={onValueChange}>
        <SelectTrigger id="playback-mode" aria-label="Playback mode">
          <SelectValue>
            {(value: string | null) =>
              value === 'normal'
                ? 'Normal'
                : value === 'dry-run'
                  ? 'Dry run'
                  : (value ?? 'Choose mode')
            }
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="normal">Normal</SelectItem>
          <SelectItem value="dry-run">Dry run</SelectItem>
          <SelectItem value="unavailable" disabled>
            Unavailable
          </SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}

describe('Base UI Select', () => {
  it('opens its listbox, selects an enabled option, and restores trigger focus', async () => {
    const onValueChange = vi.fn();
    render(<ExampleSelect onValueChange={onValueChange} />);

    const trigger = screen.getByRole('combobox', { name: 'Playback mode' });
    expect(trigger).toHaveTextContent('Normal');
    fireEvent.click(trigger);

    const listbox = screen.getByRole('listbox');
    expect(screen.getByRole('option', { name: 'Normal' })).toBeInTheDocument();
    const dryRun = screen.getByRole('option', { name: 'Dry run' });
    fireEvent.keyDown(dryRun, { key: 'Enter' });

    await waitFor(() =>
      expect(onValueChange).toHaveBeenCalledWith(
        'dry-run',
        expect.objectContaining({ reason: 'item-press' }),
      ),
    );
    await waitFor(() => expect(screen.queryByRole('listbox')).not.toBeInTheDocument());
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(listbox).not.toBeInTheDocument();
  });

  it('supports keyboard dismissal and keeps disabled options unavailable', async () => {
    const onValueChange = vi.fn();
    render(<ExampleSelect onValueChange={onValueChange} />);
    const trigger = screen.getByRole('combobox', { name: 'Playback mode' });

    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'Unavailable' })).toHaveAttribute(
      'aria-disabled',
      'true',
    );
    fireEvent.keyDown(document.activeElement ?? trigger, { key: 'Escape' });

    await waitFor(() => expect(screen.queryByRole('listbox')).not.toBeInTheDocument());
    expect(onValueChange).not.toHaveBeenCalled();
    await waitFor(() => expect(trigger).toHaveFocus());
  });
});

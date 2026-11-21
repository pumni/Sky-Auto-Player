import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AppSelect, type AppSelectOption } from './AppSelect';
import {
  findSelectedOption,
  formatFpsFrameHint,
  formatHoldFramesHint,
  formatTempoHint,
} from './appSelectHelpers';

afterEach(cleanup);

const SAMPLE_OPTIONS: AppSelectOption<'a' | 'b' | 'c'>[] = [
  { value: 'a', label: 'Alpha', triggerLabel: 'A', hint: '16.7 ms' },
  { value: 'b', label: 'Beta', hint: 'Default', swatches: ['#121314', '#b2a5e9'] },
  { value: 'c', label: 'Gamma (disabled)', disabled: true },
];

describe('appSelectHelpers', () => {
  it('formats hold-frame and fps millisecond hints accurately', () => {
    expect(formatHoldFramesHint(1, 60)).toBe('16.7 ms');
    expect(formatHoldFramesHint(1.25, 60)).toBe('20.8 ms');
    expect(formatHoldFramesHint(1.5, 60)).toBe('25.0 ms');
    expect(formatFpsFrameHint(60)).toBe('16.7 ms/f');
    expect(formatFpsFrameHint(120)).toBe('8.3 ms/f');
    expect(formatHoldFramesHint(1, 0)).toBe('');
    expect(formatFpsFrameHint(0)).toBe('');
  });

  it('marks 1x tempo as Default and resolves selected options', () => {
    expect(formatTempoHint(1)).toBe('Default');
    expect(formatTempoHint(0.95)).toBeUndefined();
    expect(findSelectedOption(SAMPLE_OPTIONS, 'b')?.label).toBe('Beta');
    expect(findSelectedOption(SAMPLE_OPTIONS, 'missing' as 'a')).toBeUndefined();
  });
});

describe('AppSelect', () => {
  it('renders a labelled combobox trigger and opens a themed listbox on click', async () => {
    const onChange = vi.fn();
    render(
      <div>
        <label htmlFor="test-base-hold">Base Hold</label>
        <AppSelect
          id="test-base-hold"
          value="a"
          options={SAMPLE_OPTIONS}
          onChange={onChange}
          listboxLabel="Base Hold"
        />
      </div>,
    );

    const combobox = screen.getByRole('combobox', { name: 'Base Hold' });
    expect(combobox).toHaveAttribute('aria-expanded', 'false');
    expect(combobox).toHaveAttribute('data-value', 'a');
    expect(combobox).toHaveTextContent('A');

    fireEvent.click(combobox);
    expect(combobox).toHaveAttribute('aria-expanded', 'true');

    const listbox = screen.getByRole('listbox', { name: 'Base Hold' });
    expect(listbox).toBeInTheDocument();
    expect(screen.getByRole('option', { name: /Alpha/ })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByText('16.7 ms')).toBeInTheDocument();
    expect(screen.getByText('Default')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('option', { name: /Beta/ }));
    expect(onChange).toHaveBeenCalledWith('b');
    await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
  });

  it('does not select disabled options and closes cleanly on Escape', async () => {
    const onChange = vi.fn();
    render(
      <AppSelect
        value="a"
        options={SAMPLE_OPTIONS}
        onChange={onChange}
        ariaLabel="Mode"
        listboxLabel="Mode"
      />,
    );

    const combobox = screen.getByRole('combobox', { name: 'Mode' });
    fireEvent.keyDown(combobox, { key: 'ArrowDown' });
    const listbox = screen.getByRole('listbox', { name: 'Mode' });
    expect(listbox).toBeInTheDocument();

    const disabledOption = screen.getByRole('option', { name: /Gamma/ });
    expect(disabledOption).toHaveAttribute('aria-disabled', 'true');
    fireEvent.click(disabledOption);
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.keyDown(listbox, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
    await waitFor(() => expect(combobox).toHaveFocus());
  });
});

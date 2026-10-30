import { beforeEach, describe, expect, it } from 'vitest';
import { applyAppearance } from './applyAppearance';

describe('applyAppearance', () => {
  beforeEach(() => {
    document.documentElement.removeAttribute('data-palette');
    document.documentElement.removeAttribute('data-color-mode');
    document.documentElement.dataset.theme = 'stale';
  });

  it('applies the resolved mode and saved dark palette while removing legacy theme state', () => {
    applyAppearance({ colorMode: 'light', palette: 'slate' });

    expect(document.documentElement.dataset.palette).toBe('slate');
    expect(document.documentElement.dataset.colorMode).toBe('light');
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});

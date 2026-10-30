import { describe, expect, it } from 'vitest';
import { resolveAppearance } from './resolveAppearance';

describe('resolveAppearance', () => {
  it('follows the supplied system mode and retains the dark palette', () => {
    expect(resolveAppearance('system', 'light', 'slate')).toEqual({
      colorMode: 'light',
      palette: 'slate',
    });
    expect(resolveAppearance('system', 'dark', 'aurora')).toEqual({
      colorMode: 'dark',
      palette: 'aurora',
    });
  });

  it('lets explicit Light and Dark ignore the system mode', () => {
    expect(resolveAppearance('light', 'dark', 'graphite').colorMode).toBe('light');
    expect(resolveAppearance('dark', 'light', 'classic').colorMode).toBe('dark');
  });
});

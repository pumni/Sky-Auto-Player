import type { AppearanceMode, PaletteId } from '../bridge/DesktopBridge';

export type ColorMode = 'light' | 'dark';

export interface EffectiveAppearance {
  colorMode: ColorMode;
  palette: PaletteId;
}

export function resolveAppearance(
  mode: AppearanceMode,
  systemMode: ColorMode,
  darkPalette: PaletteId,
): EffectiveAppearance {
  return {
    colorMode: mode === 'system' ? systemMode : mode,
    palette: darkPalette,
  };
}

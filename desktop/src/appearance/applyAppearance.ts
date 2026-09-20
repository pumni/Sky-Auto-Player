import type { PaletteId } from '../bridge/DesktopBridge';

export function applyAppearance(palette: PaletteId): void {
  const root = document.documentElement;
  root.dataset.palette = palette;
  root.dataset.colorMode = 'dark';
  delete root.dataset.theme;
}

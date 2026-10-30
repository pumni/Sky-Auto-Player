import type { EffectiveAppearance } from './resolveAppearance';

export function applyAppearance(appearance: EffectiveAppearance): void {
  const root = document.documentElement;
  root.dataset.palette = appearance.palette;
  root.dataset.colorMode = appearance.colorMode;
  delete root.dataset.theme;
}

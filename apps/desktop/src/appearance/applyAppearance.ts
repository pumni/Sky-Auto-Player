import type { EffectiveAppearance } from './resolveAppearance';

export function applyAppearance(appearance: EffectiveAppearance): void {
  const root = document.documentElement;
  const changed =
    root.dataset.palette !== appearance.palette || root.dataset.colorMode !== appearance.colorMode;
  if (changed) root.dataset.themeApplying = '';
  root.dataset.palette = appearance.palette;
  root.dataset.colorMode = appearance.colorMode;
  delete root.dataset.theme;
  if (changed) {
    // Force a single synchronous style flush so M15 theme switches apply in 0ms
    // across all child elements without leaving transitions disabled afterward.
    void root.offsetHeight;
    delete root.dataset.themeApplying;
  }
}

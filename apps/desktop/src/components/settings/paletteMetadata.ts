import type { PaletteId } from '../../bridge/DesktopBridge';

export interface PaletteMeta {
  id: PaletteId;
  label: string;
  swatches: readonly [string, string, string, string];
}

export const DEFAULT_PALETTE: PaletteMeta = {
  id: 'graphite',
  label: 'Graphite (Default)',
  swatches: ['#121314', '#191a1b', '#202122', '#8e9dff'],
};

export const PALETTES: readonly PaletteMeta[] = [
  DEFAULT_PALETTE,
  { id: 'aurora', label: 'Aurora', swatches: ['#0b1018', '#101722', '#151e2b', '#785ddd'] },
  { id: 'minimalist', label: 'Minimalist', swatches: ['#17191d', '#202328', '#292d33', '#a0b8d5'] },
  { id: 'slate', label: 'Slate', swatches: ['#10171d', '#17232c', '#20313d', '#6ca7d8'] },
  { id: 'cyberpunk', label: 'Cyberpunk', swatches: ['#130f1d', '#1d1529', '#291c37', '#d95abf'] },
  { id: 'classic', label: 'Classic', swatches: ['#171512', '#211d18', '#2c261f', '#c99242'] },
];

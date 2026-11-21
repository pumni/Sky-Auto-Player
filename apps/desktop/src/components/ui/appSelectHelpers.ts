export interface AppSelectOption<T extends string = string> {
  value: T;
  label: string;
  triggerLabel?: string;
  hint?: string;
  swatches?: readonly string[];
  disabled?: boolean;
}

export function formatHoldFramesHint(holdFrames: number, fps: number): string {
  if (!Number.isFinite(holdFrames) || !Number.isFinite(fps) || fps <= 0) return '';
  const frameUs = Math.ceil(1_000_000 / fps);
  const holdMs = Math.ceil(holdFrames * frameUs) / 1_000;
  return `${holdMs.toFixed(1)} ms`;
}

export function formatFpsFrameHint(fps: number): string {
  if (!Number.isFinite(fps) || fps <= 0) return '';
  const frameMs = Math.ceil(1_000_000 / fps) / 1_000;
  return `${frameMs.toFixed(1)} ms/f`;
}

export function formatTempoHint(tempoScale: number): string | undefined {
  return Math.abs(tempoScale - 1) < 1e-6 ? 'Default' : undefined;
}

export function findSelectedOption<T extends string>(
  options: readonly AppSelectOption<T>[],
  value: T,
): AppSelectOption<T> | undefined {
  return options.find((option) => option.value === value);
}

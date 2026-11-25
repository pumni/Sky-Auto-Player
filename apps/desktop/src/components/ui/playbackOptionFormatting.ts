export interface PlaybackHoldTiming {
  frameUs: number;
  frameBaseHoldUs: number;
  minHoldUs: number;
}

export function computePlaybackHoldTiming(
  fps: number,
  holdFrames: number,
  timingMarginUs: number,
): PlaybackHoldTiming {
  const frameUs = Math.ceil(1_000_000 / fps);
  const frameBaseHoldUs = Math.ceil(holdFrames * frameUs);
  const minHoldUs = frameBaseHoldUs + timingMarginUs;
  return { frameUs, frameBaseHoldUs, minHoldUs };
}

export function formatHoldFramesHint(holdFrames: number, fps: number): string {
  if (!Number.isFinite(holdFrames) || !Number.isFinite(fps) || fps <= 0) return '';
  const { frameBaseHoldUs } = computePlaybackHoldTiming(fps, holdFrames, 0);
  return `${(frameBaseHoldUs / 1_000).toFixed(1)} ms`;
}

export function formatFpsFrameHint(fps: number): string {
  if (!Number.isFinite(fps) || fps <= 0) return '';
  const frameMs = Math.ceil(1_000_000 / fps) / 1_000;
  return `${frameMs.toFixed(1)} ms/f`;
}

export function formatTempoHint(tempoScale: number): string | undefined {
  return Math.abs(tempoScale - 1) < 1e-6 ? 'Default' : undefined;
}

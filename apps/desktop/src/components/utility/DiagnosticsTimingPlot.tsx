import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { TabsContent } from '../ui/tabs';
import { number } from './diagnosticsPresentation';
import { DiagnosticsEmptyState } from './DiagnosticsReadouts';

export function DiagnosticsTimingPlot({
  samples,
  hasActiveSession,
  dryRun,
}: {
  samples: DesktopStore['diagnostics']['samples'];
  hasActiveSession: boolean;
  dryRun: boolean;
}) {
  const width = 300;
  const height = 140;
  const availableSamples = samples.filter(
    (sample) =>
      sample.backend_status !== 'unavailable' &&
      sample.sender_sample_count > 0 &&
      sample.max_sendinput_pre_call_lateness_us !== null,
  );
  if (!hasActiveSession) {
    return (
      <DiagnosticsEmptyState
        title="No active playback session"
        detail="Sender-side timing samples appear when playback starts."
      />
    );
  }
  if (dryRun) {
    return (
      <DiagnosticsEmptyState
        title="No physical timing samples in dry run"
        detail="Dry-run playback sends no physical input, so sender timing is not measured."
      />
    );
  }
  if (samples.length === 0) {
    return (
      <DiagnosticsEmptyState
        title="Waiting for session diagnostics"
        detail="Native playback is active; its matching diagnostics snapshot has not arrived yet."
      />
    );
  }
  if (availableSamples.length === 0) {
    const latest = samples.at(-1);
    const playerAttached = latest?.player_attached === true;
    const backendUnavailable = latest === undefined || latest.backend_status === 'unavailable';
    const physicalPlayerMissing = latest?.physical_session === true && !playerAttached;
    const backendError = latest?.backend_status === 'error';
    const noSenderSamples =
      !backendUnavailable &&
      !physicalPlayerMissing &&
      !backendError &&
      playerAttached &&
      latest.sender_sample_count === 0;
    return (
      <DiagnosticsEmptyState
        title={
          backendUnavailable
            ? 'Timing unavailable'
            : physicalPlayerMissing
              ? 'Player not attached'
              : backendError
                ? 'Sender timing error'
                : noSenderSamples
                  ? 'No sender samples yet'
                  : 'Timing unavailable'
        }
        detail={
          backendUnavailable
            ? latest === undefined
              ? 'No sender diagnostics snapshot is available for this session yet.'
              : 'Sender-side backend metrics are unavailable for this dispatch profile.'
            : physicalPlayerMissing
              ? 'A physical session is active, but its native player is not attached.'
              : backendError
                ? `The sender backend reported an error: ${latest?.last_error ?? 'no sender sample is available.'}`
                : noSenderSamples
                  ? 'The physical player is attached; no SendInput call has been sampled yet.'
                  : 'Sender-side timing measurements are unavailable for this session.'
        }
      />
    );
  }
  const formatAxisUs = (valueUs: number): string => {
    if (valueUs === 0) return '0';
    if (valueUs >= 1_000) return `${(valueUs / 1_000).toFixed(1)} ms`;
    return `${Math.round(valueUs)} µs`;
  };
  const values = availableSamples.map((sample) => sample.max_sendinput_pre_call_lateness_us!);
  const minimum = 0;
  const maximum = Math.max(0, ...values);
  const scaleMax = Math.max(100, Math.ceil(maximum / 100) * 100);
  const range = Math.max(1, scaleMax - minimum);
  const plotLeft = 48;
  const plotRight = width - 10;
  const plotTop = 14;
  const plotBottom = height - 14;
  const plotWidth = plotRight - plotLeft;
  const yFor = (value: number) => plotBottom - ((value - minimum) / range) * (plotBottom - plotTop);
  const coords = values.map((value, index) => {
    const x =
      values.length <= 1
        ? plotLeft + plotWidth / 2
        : plotLeft + (index / (values.length - 1)) * plotWidth;
    return { x, y: yFor(value) };
  });
  const points = coords.map((point) => `${point.x.toFixed(1)},${point.y.toFixed(1)}`).join(' ');
  const firstPoint = coords[0];
  const lastPoint = coords[coords.length - 1];
  const areaPoints =
    coords.length >= 2 && firstPoint && lastPoint
      ? `${firstPoint.x.toFixed(1)},${plotBottom.toFixed(1)} ${points} ${lastPoint.x.toFixed(1)},${plotBottom.toFixed(1)}`
      : '';
  const latest = values.length ? values[values.length - 1] : null;
  const zeroY = yFor(0);
  const midValue = scaleMax / 2;
  const midY = yFor(midValue);
  const maxY = yFor(scaleMax);
  const latestSample = availableSamples[availableSamples.length - 1];
  return (
    <figure className="my-2 grid min-w-0 gap-1">
      <svg
        className="max-h-40 w-full overflow-visible"
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-labelledby="timing-plot-title timing-plot-description"
      >
        <title id="timing-plot-title">
          Session max SendInput pre-call lateness observed at each diagnostics snapshot
        </title>
        <desc id="timing-plot-description">
          Cumulative session maximum observed at each diagnostics snapshot; it does not decrease
          after recovery. Completion latency is reported separately.
        </desc>
        <line
          x1={plotLeft}
          y1={maxY}
          x2={plotRight}
          y2={maxY}
          className="stroke-border stroke-[1]"
        />
        <line
          x1={plotLeft}
          y1={midY}
          x2={plotRight}
          y2={midY}
          className="stroke-border stroke-[1]"
        />
        <line
          x1={plotLeft}
          y1={zeroY}
          x2={plotRight}
          y2={zeroY}
          className="stroke-text-tertiary stroke-[1]"
        />
        <text
          x={plotLeft - 8}
          y={maxY + 4}
          textAnchor="end"
          className="fill-text-tertiary text-[10px]"
        >
          {formatAxisUs(scaleMax)}
        </text>
        <text
          x={plotLeft - 8}
          y={midY + 4}
          textAnchor="end"
          className="fill-text-tertiary text-[10px]"
        >
          {formatAxisUs(midValue)}
        </text>
        <text
          x={plotLeft - 8}
          y={zeroY + 4}
          textAnchor="end"
          className="fill-text-tertiary text-[10px]"
        >
          0
        </text>
        {areaPoints && <polygon points={areaPoints} className="fill-accent-ink/10" />}
        {points && <polyline points={points} className="fill-none stroke-accent-ink stroke-2" />}
        {lastPoint && (
          <circle
            cx={lastPoint.x}
            cy={lastPoint.y}
            r="3.5"
            className="fill-accent-ink stroke-card stroke-[1.5]"
          />
        )}
      </svg>
      <figcaption className="mt-1.5 text-xs text-text-tertiary">
        {latest === null
          ? 'No sender-side timing samples yet.'
          : `Session max pre-call lateness observed at the latest diagnostics snapshot: ${latest} μs across ${values.length} snapshots. This cumulative value does not decrease after recovery.${
              latestSample?.p95_ms === null || latestSample?.p95_ms === undefined
                ? ''
                : ` Completion p95 observer value ${number(latestSample.p95_ms)} ms.`
            }`}
      </figcaption>
    </figure>
  );
}

export function DiagnosticsTimingPanel({ useStore }: { useStore: DesktopStoreHook }) {
  const samples = useStore((store) => store.diagnostics.samples);
  const diagnosticsEnabled = useStore((store) => store.diagnostics.enabled);
  const sessionId = useStore((store) => store.playback.sessionId);
  const playbackState = useStore((store) => store.playback.state);
  const dryRun = useStore(
    (store) => store.playback.context?.dryRun ?? store.playback.prepared?.config.dry_run ?? false,
  );
  const playbackIsActive = ['starting', 'playing', 'paused', 'stopping'].includes(playbackState);
  const nativeSessionActive = diagnosticsEnabled && playbackIsActive && sessionId !== null;
  const latest = samples[samples.length - 1];
  const activeSession = nativeSessionActive && latest?.session_id === sessionId;

  return (
    <TabsContent value="timing" className="min-h-[116px] min-w-0 flex-1 pt-3">
      <DiagnosticsTimingPlot
        samples={activeSession ? samples : []}
        hasActiveSession={nativeSessionActive}
        dryRun={nativeSessionActive && dryRun}
      />
    </TabsContent>
  );
}

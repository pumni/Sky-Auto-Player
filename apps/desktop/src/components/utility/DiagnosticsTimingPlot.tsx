import { TabPanel } from 'react-aria-components';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
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
  const width = 560;
  const height = 132;
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
  const values = availableSamples.map((sample) => sample.max_sendinput_pre_call_lateness_us!);
  const minimum = 0;
  const maximum = Math.max(0, ...values);
  const range = Math.max(1, maximum - minimum);
  const plotTop = 4;
  const plotBottom = height - 4;
  const yFor = (value: number) => plotBottom - ((value - minimum) / range) * (plotBottom - plotTop);
  const points = values
    .map((value, index) => {
      const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * width;
      return `${x.toFixed(1)},${yFor(value).toFixed(1)}`;
    })
    .join(' ');
  const latest = values.length ? values[values.length - 1] : null;
  const zeroY = yFor(0);
  const latestSample = availableSamples[availableSamples.length - 1];
  return (
    <figure className="diagnostics-plot">
      <svg
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
        <line x1="0" y1={zeroY} x2={width} y2={zeroY} className="plot-zero-axis" />
        {points && <polyline points={points} className="plot-line" />}
      </svg>
      <figcaption>
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
    <TabPanel id="timing" className="diagnostics-panel">
      <DiagnosticsTimingPlot
        samples={activeSession ? samples : []}
        hasActiveSession={nativeSessionActive}
        dryRun={nativeSessionActive && dryRun}
      />
    </TabPanel>
  );
}

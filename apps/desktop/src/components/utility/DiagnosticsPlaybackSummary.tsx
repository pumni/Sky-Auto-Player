import type { DesktopStoreHook } from '../../state/store';
import { playbackSessionStatusConcern } from '../../state/store';
import { TEST_MODE_DIAGNOSTICS_LABEL } from '../player/playerFormatting';
import { playbackStatusLabel } from './diagnosticsPresentation';

export function DiagnosticsPlaybackSummary({ useStore }: { useStore: DesktopStoreHook }) {
  const error = useStore((store) => store.playback.error);
  const sessionId = useStore((store) => store.playback.sessionId);
  const startRequestId = useStore((store) => store.playback.startRequestId);
  const state = useStore((store) => store.playback.state);
  const statusRetryPending = useStore((store) => store.playback.statusRetryPending);
  const transportOperation = useStore((store) => store.playback.transportOperation);
  const prepared = useStore((store) => store.playback.prepared);
  const currentSongId = useStore((store) => store.playback.currentSong?.songId ?? null);
  const currentSongTitle = useStore((store) => store.playback.currentSong?.title);
  const hasCurrentSong = useStore((store) => store.playback.currentSong !== null);
  const confirmedDryRun = useStore((store) => store.playback.context?.dryRun ?? null);
  const preparedDryRun = useStore(
    (store) =>
      store.playback.preparedContext?.dryRun ?? store.playback.prepared?.config.dry_run ?? null,
  );
  const playback = {
    error,
    sessionId,
    startRequestId,
    state,
    statusRetryPending,
    transportOperation,
    prepared,
  };
  const status = playbackStatusLabel(playback);
  const isActive = ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
  const sessionStatusConcern = playbackSessionStatusConcern(playback);
  const modeSummary =
    (isActive || sessionStatusConcern !== null) && confirmedDryRun !== null
      ? {
          label: 'Mode',
          value: confirmedDryRun ? TEST_MODE_DIAGNOSTICS_LABEL : 'Physical input',
        }
      : preparedDryRun !== null && (isActive || playback.prepared !== null)
        ? {
            label: 'Prepared mode',
            value: preparedDryRun ? TEST_MODE_DIAGNOSTICS_LABEL : 'Physical input',
          }
        : null;
  const hasPlaybackContext =
    playback.state !== 'idle' ||
    playback.transportOperation !== null ||
    playback.startRequestId !== null ||
    playback.prepared !== null ||
    hasCurrentSong ||
    playback.error !== null;

  if (!hasPlaybackContext) return null;

  return (
    <section
      data-slot="diagnostics-playback-summary"
      className="mb-3 grid min-w-0 gap-1 border-b border-border pb-2.5"
      aria-label="Playback identity and status"
    >
      <strong className="text-[13px] font-semibold text-foreground">
        Playback status: {status}
      </strong>
      {hasCurrentSong && (
        <span className="break-words text-xs text-text-secondary">
          {sessionStatusConcern !== null
            ? 'Last confirmed session song'
            : isActive
              ? 'Now Playing'
              : 'Last session song'}
          : {currentSongTitle}
        </span>
      )}
      {playback.prepared?.song && playback.prepared.song.song_id !== currentSongId && (
        <span className="break-words text-xs text-text-secondary">
          Prepared: {playback.prepared.song.title}
        </span>
      )}
      {modeSummary && (
        <span className="break-words text-xs text-text-secondary">
          {modeSummary.label}: {modeSummary.value}
        </span>
      )}
    </section>
  );
}

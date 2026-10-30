import type { DesktopStoreHook } from '../../state/store';
import { playbackSessionStatusConcern } from '../../state/store';
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
  const dryRun = useStore(
    (store) => store.playback.context?.dryRun ?? store.playback.prepared?.config.dry_run ?? null,
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
  const hasPlaybackContext =
    playback.state !== 'idle' ||
    playback.transportOperation !== null ||
    playback.startRequestId !== null ||
    playback.prepared !== null ||
    hasCurrentSong ||
    playback.error !== null;

  if (!hasPlaybackContext) return null;

  return (
    <section className="diagnostics-playback-summary" aria-label="Playback identity and status">
      <strong>Playback status: {status}</strong>
      {hasCurrentSong && (
        <span>
          {sessionStatusConcern !== null
            ? 'Last confirmed session song'
            : isActive
              ? 'Now Playing'
              : 'Last session song'}
          : {currentSongTitle}
        </span>
      )}
      {playback.prepared?.song && playback.prepared.song.song_id !== currentSongId && (
        <span>Prepared: {playback.prepared.song.title}</span>
      )}
      {dryRun !== null && (isActive || playback.prepared) && (
        <span>Mode: {dryRun ? 'Dry run · no physical input sent' : 'Physical input'}</span>
      )}
    </section>
  );
}

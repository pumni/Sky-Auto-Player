import { memo, type CSSProperties, type ReactNode } from 'react';
import { Volume2 } from 'lucide-react';
import type { SongRow } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../../state/store';
import { selectNowPlayingSongId, selectRowAtIndex } from '../../state/store';
import { TrackActionsMenu } from './TrackActionsMenu';

interface TrackRowProps {
  index: number;
  ordinal?: number;
  row: SongRow;
  selected: boolean;
  playing?: boolean;
  start: number;
  onFocus?: () => void;
  onSelect: () => void;
  onToggleLiked?: () => void;
  actions?: ReactNode;
}

export function formatDuration(durationUs: number | null): string {
  if (durationUs === null) return '—';
  const seconds = Math.max(0, Math.round(durationUs / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

function metadataStatus(state: SongRow['metadata_state']): {
  label: string;
  value: string;
} {
  switch (state) {
    case 'pending':
      return { label: 'Sheet analysis loading', value: '…' };
    case 'unsupported':
      return { label: 'Unsupported sheet format', value: 'Unsupported' };
    case 'malformed':
      return { label: 'Malformed sheet', value: 'Invalid' };
    case 'unplayable':
      return { label: 'No playable Piano notes', value: 'Unplayable' };
    case 'requires_confirmation':
      return { label: 'Conversion review required', value: 'Review' };
    case 'physical_risk':
      return { label: 'Physical timing risk', value: 'Risk' };
    case 'error':
      return { label: 'Sheet metadata unavailable', value: 'Unavailable' };
    case 'ready':
      return { label: 'Sheet ready', value: 'Ready' };
  }
}

function isInteractiveRowTarget(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    target.closest('button, a[href], input, select, textarea, [contenteditable="true"]') !== null
  );
}

export const TrackRow = memo(function TrackRow({
  index,
  ordinal = index + 1,
  row,
  selected,
  playing = false,
  start,
  onFocus,
  onSelect,
  onToggleLiked,
  actions,
}: TrackRowProps) {
  const style: CSSProperties = { transform: `translateY(${start}px)` };
  const metadata = row.metadata_state === 'ready' ? null : metadataStatus(row.metadata_state);
  return (
    <div
      id={`song-row-${row.song_id}`}
      className={`track-table-row track-row${selected ? ' is-selected' : ''}${playing ? ' is-playing' : ''}`}
      role="row"
      aria-selected={selected}
      aria-current={playing ? 'true' : undefined}
      aria-rowindex={index + 2}
      tabIndex={-1}
      style={style}
      onFocus={onFocus}
      onClick={(event) => {
        if (isInteractiveRowTarget(event.target)) return;
        event.currentTarget.closest<HTMLElement>('[role="grid"]')?.focus({ preventScroll: true });
        onSelect();
      }}
    >
      <span className="track-cell track-cell-index" role="gridcell">
        {playing ? <Volume2 size={14} aria-hidden="true" /> : ordinal}
      </span>
      <span className="track-cell track-cell-title" role="gridcell" title={row.title}>
        <span className="track-title-text">{row.title}</span>
        {selected && <span className="visually-hidden">Selected for inspection</span>}
        {playing && <span className="visually-hidden">Now Playing</span>}
        {actions}
      </span>
      <span className="track-cell track-cell-liked" role="gridcell">
        <button
          className={`track-like-button${row.liked ? ' is-liked' : ''}`}
          type="button"
          aria-label={
            row.liked ? `Remove ${row.title} from Liked Songs` : `Add ${row.title} to Liked Songs`
          }
          aria-pressed={row.liked}
          title={row.liked ? 'Remove from Liked Songs' : 'Add to Liked Songs'}
          onClick={(event) => {
            event.stopPropagation();
            onToggleLiked?.();
          }}
          onKeyDown={(event) => event.stopPropagation()}
        >
          <span aria-hidden="true">{row.liked ? '♥' : '♡'}</span>
        </button>
      </span>
      <span className="track-cell track-cell-notes" role="gridcell">
        {metadata ? (
          <span
            className={`track-metadata-status is-${row.metadata_state}`}
            aria-label={metadata.label}
            title={metadata.label}
          >
            {metadata.value}
          </span>
        ) : (
          (row.note_count ?? '—')
        )}
      </span>
      <span className="track-cell track-cell-duration" role="gridcell">
        {metadata ? (
          <span
            className={`track-metadata-status is-${row.metadata_state}`}
            aria-label={metadata.label}
            title={metadata.label}
          >
            {metadata.value}
          </span>
        ) : (
          formatDuration(row.duration_us)
        )}
      </span>
    </div>
  );
});

interface VirtualTrackRowProps {
  index: number;
  sourceIndex?: number;
  start: number;
  useStore: DesktopStoreHook;
}

export const VirtualTrackRow = memo(function VirtualTrackRow({
  index,
  sourceIndex = index,
  start,
  useStore,
}: VirtualTrackRowProps) {
  const row = useStore((store) => selectRowAtIndex(store.library, sourceIndex));
  const selected = useStore(
    (store) =>
      selectRowAtIndex(store.library, sourceIndex)?.song_id === store.library.selectedSongId,
  );
  const playing = useStore(
    (store) =>
      selectRowAtIndex(store.library, sourceIndex)?.song_id === selectNowPlayingSongId(store),
  );
  const setSongLiked = useStore((store) => store.setSongLiked);
  const selectSong = useStore((store) => store.selectSong);

  if (!row) {
    return (
      <div
        className="track-table-row track-row track-row-placeholder"
        role="row"
        aria-busy="true"
        aria-rowindex={index + 2}
        style={{ transform: `translateY(${start}px)` }}
      >
        <span className="track-cell track-cell-index" role="gridcell">
          {sourceIndex + 1}
        </span>
        <span className="track-cell track-cell-title" role="gridcell">
          Loading song…
        </span>
      </div>
    );
  }

  return (
    <TrackRow
      row={row}
      index={index}
      ordinal={sourceIndex + 1}
      selected={selected}
      playing={playing}
      start={start}
      onSelect={() => void selectSong(row.song_id)}
      onToggleLiked={() => void setSongLiked(row.song_id, !row.liked)}
      actions={<TrackActionsMenu row={row} useStore={useStore} />}
    />
  );
});

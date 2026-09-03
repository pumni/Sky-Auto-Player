import { memo, type CSSProperties } from 'react';
import type { SongRow } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../../state/store';
import { selectRowAtIndex } from '../../state/store';

interface TrackRowProps {
  index: number;
  row: SongRow;
  selected: boolean;
  start: number;
  onFocus?: () => void;
  onSelect: () => void;
  onToggleLiked?: () => void;
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
  return state === 'pending'
    ? { label: 'Metadata loading', value: '…' }
    : { label: 'Metadata unavailable', value: 'Unavailable' };
}

export const TrackRow = memo(function TrackRow({
  index,
  row,
  selected,
  start,
  onFocus,
  onSelect,
  onToggleLiked,
}: TrackRowProps) {
  const style: CSSProperties = { transform: `translateY(${start}px)` };
  const metadata = row.metadata_state === 'ready' ? null : metadataStatus(row.metadata_state);
  return (
    <div
      id={`song-row-${row.song_id}`}
      className={`track-table-row track-row${selected ? ' is-selected' : ''}`}
      role="row"
      aria-selected={selected}
      aria-rowindex={index + 2}
      tabIndex={-1}
      style={style}
      onFocus={onFocus}
      onClick={onSelect}
    >
      <span className="track-cell track-cell-index" role="gridcell">
        {index + 1}
      </span>
      <span className="track-cell track-cell-title" role="gridcell" title={row.title}>
        {row.title}
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
  start: number;
  useStore: DesktopStoreHook;
}

export const VirtualTrackRow = memo(function VirtualTrackRow({
  index,
  start,
  useStore,
}: VirtualTrackRowProps) {
  const row = useStore((store) => selectRowAtIndex(store.library, index));
  const selected = useStore(
    (store) => selectRowAtIndex(store.library, index)?.song_id === store.library.selectedSongId,
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
          {index + 1}
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
      selected={selected}
      start={start}
      onSelect={() => void selectSong(row.song_id)}
      onToggleLiked={() => void setSongLiked(row.song_id, !row.liked)}
    />
  );
});

import { memo, useState, type CSSProperties, type ReactNode } from 'react';
import { Focusable } from 'react-aria-components';
import { Tooltip, TooltipTrigger } from 'react-aria-components/Tooltip';
import { Heart, Music2 } from 'lucide-react';
import type { SongRow } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../../state/store';
import { selectNowPlayingSongId, selectRowAtIndex } from '../../state/store';
import { StatusBadge, type StatusBadgeTone } from '../ui/StatusBadge';
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
  tone: StatusBadgeTone;
} {
  switch (state) {
    case 'pending':
      return { label: 'Metadata loading', value: 'Loading', tone: 'neutral' };
    case 'unsupported':
      return { label: 'Unsupported sheet format', value: 'Unsupported', tone: 'danger' };
    case 'malformed':
      return { label: 'Malformed sheet', value: 'Invalid', tone: 'danger' };
    case 'unplayable':
      return { label: 'No playable Piano notes', value: 'Unplayable', tone: 'danger' };
    case 'requires_confirmation':
      return { label: 'Conversion review required', value: 'Review', tone: 'warning' };
    case 'physical_risk':
      return { label: 'Physical timing risk', value: 'Risk', tone: 'warning' };
    case 'error':
      return { label: 'Metadata unavailable', value: 'Unavailable', tone: 'danger' };
    case 'ready':
      return { label: 'Sheet ready', value: 'Ready', tone: 'neutral' };
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
  const [clippedHoverSongId, setClippedHoverSongId] = useState<string | null>(null);
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
        {playing ? <Music2 size={14} aria-hidden="true" /> : ordinal}
      </span>
      <TooltipTrigger
        isOpen={clippedHoverSongId === row.song_id}
        onOpenChange={(isOpen) => {
          if (!isOpen) setClippedHoverSongId(null);
        }}
      >
        <Focusable>
          <span className="track-cell track-cell-title" role="gridcell" tabIndex={-1}>
            <span
              className="track-title-text"
              onPointerEnter={(event) => {
                // Preserve the grid's one-focus-target keyboard model. The existing
                // gridcell is the tooltip anchor; text clipping alone controls opening.
                if (event.pointerType !== 'mouse' && event.pointerType !== 'pen') return;
                const element = event.currentTarget;
                setClippedHoverSongId(
                  element.scrollWidth > element.clientWidth ? row.song_id : null,
                );
              }}
              onPointerLeave={() => setClippedHoverSongId(null)}
            >
              {row.title}
            </span>
            {metadata && (
              <StatusBadge
                className="track-metadata-status"
                tone={metadata.tone}
                aria-label={metadata.label}
                title={metadata.label}
              >
                {metadata.value}
              </StatusBadge>
            )}
            {selected && <span className="visually-hidden">Selected for inspection</span>}
            {playing && <span className="visually-hidden">Now Playing</span>}
            {actions}
          </span>
        </Focusable>
        <Tooltip className="track-title-tooltip" placement="top start">
          {row.title}
        </Tooltip>
      </TooltipTrigger>
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
          <Heart size={16} fill={row.liked ? 'currentColor' : 'none'} aria-hidden="true" />
        </button>
      </span>
      <span className="track-cell track-cell-notes" role="gridcell">
        {metadata ? '—' : (row.note_count ?? '—')}
      </span>
      <span className="track-cell track-cell-duration" role="gridcell">
        {metadata ? '—' : formatDuration(row.duration_us)}
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

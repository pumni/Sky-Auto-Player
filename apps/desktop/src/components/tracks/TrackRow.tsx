import { memo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { Heart, Music2 } from 'lucide-react';
import type { SongRow } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../../state/store';
import { selectNowPlayingSongId, selectRowAtIndex } from '../../state/store';
import { StatusBadge, type StatusBadgeTone } from '../ui/StatusBadge';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/tooltip';
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
    target.closest(
      'button, a[href], input, select, textarea, [contenteditable="true"], [role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"], [role="menu"], [role="dialog"], [role="listbox"], [role="option"]',
    ) !== null
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
  const titleTextRef = useRef<HTMLSpanElement>(null);
  const style: CSSProperties = { transform: `translateY(${start}px)` };
  const metadata = row.metadata_state === 'ready' ? null : metadataStatus(row.metadata_state);
  return (
    <div
      id={`song-row-${row.song_id}`}
      className={`group absolute inset-x-1.5 grid h-[46px] min-w-0 grid-cols-[36px_minmax(0,1fr)_32px_72px_76px] items-center gap-2 rounded-control border-l-2 px-[10px] pr-3 text-[13px] font-medium outline-none transition-colors duration-fast hover:bg-row-hover focus-visible:ring-2 focus-visible:ring-ring @max-[520px]/track-browser:right-1 @max-[520px]/track-browser:grid-cols-[30px_minmax(0,1fr)_32px_76px] @max-[520px]/track-browser:px-1 @max-[520px]/track-browser:pr-2 @max-[440px]/track-browser:left-1 @max-[440px]/track-browser:right-1 @max-[440px]/track-browser:grid-cols-[26px_minmax(0,1fr)_32px_70px] @max-[440px]/track-browser:gap-[5px] @max-[440px]/track-browser:pl-0.5 @max-[440px]/track-browser:pr-1 motion-reduce:transition-none ${selected ? 'border-l-accent-ink' : 'border-l-transparent'} ${selected ? 'bg-selection font-semibold text-foreground' : playing ? 'bg-playing text-text-secondary' : 'text-text-secondary'} data-[state=selected]:hover:bg-selection data-[state=selected]:focus-visible:bg-selection data-[state=playing]:hover:bg-playing data-[state=playing]:focus-visible:bg-playing data-[state=selected-playing]:hover:bg-selection data-[state=selected-playing]:focus-visible:bg-selection`}
      data-selected={selected}
      data-playing={playing}
      data-state={
        selected && playing
          ? 'selected-playing'
          : selected
            ? 'selected'
            : playing
              ? 'playing'
              : 'idle'
      }
      data-slot="track-row"
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
      <span
        className={`min-w-0 text-center tabular-nums ${playing ? 'grid place-items-center font-semibold text-accent-ink' : 'text-text-tertiary'}`}
        role="gridcell"
      >
        {playing ? <Music2 size={14} aria-hidden="true" /> : ordinal}
      </span>
      <span
        data-slot="track-title-cell"
        className="flex min-w-0 items-center gap-1.5 overflow-hidden"
        role="gridcell"
      >
        <Tooltip
          open={clippedHoverSongId === row.song_id}
          onOpenChange={(isOpen) => {
            if (!isOpen) {
              setClippedHoverSongId(null);
            } else {
              const title = titleTextRef.current;
              setClippedHoverSongId(
                title && title.scrollWidth > title.clientWidth ? row.song_id : null,
              );
            }
          }}
        >
          <TooltipTrigger
            render={
              <span
                ref={titleTextRef}
                data-track-title
                className={`min-w-0 flex-[0_1_auto] overflow-hidden text-ellipsis whitespace-nowrap ${playing ? 'font-semibold text-accent-ink' : ''}`}
              />
            }
            onPointerEnter={(event) => {
              if (event.pointerType !== 'mouse' && event.pointerType !== 'pen') return;
              const title = event.currentTarget;
              setClippedHoverSongId(title.scrollWidth > title.clientWidth ? row.song_id : null);
            }}
            onPointerLeave={() => setClippedHoverSongId(null)}
          >
            {row.title}
          </TooltipTrigger>
          <TooltipContent side="top" align="start">
            {row.title}
          </TooltipContent>
        </Tooltip>
        {metadata && (
          <StatusBadge
            className="shrink-0"
            tone={metadata.tone}
            aria-label={metadata.label}
            title={metadata.label}
          >
            {metadata.value}
          </StatusBadge>
        )}
        {selected && <span className="sr-only">Selected for inspection</span>}
        {playing && <span className="sr-only">Now Playing</span>}
        {actions}
      </span>
      <span className="flex min-w-0 items-center justify-center" role="gridcell">
        <Button
          variant="ghost"
          size="icon-sm"
          className={`size-7 rounded-full p-0 text-text-tertiary opacity-0 transition-colors duration-fast group-hover:opacity-100 group-focus-within:opacity-100 group-data-[selected=true]:opacity-100 group-data-[playing=true]:opacity-100 hover:bg-control-hover hover:text-accent-ink focus-visible:opacity-100 focus-visible:text-accent-ink ${row.liked ? 'bg-transparent text-accent-ink opacity-100' : ''}`}
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
        </Button>
      </span>
      <span
        className="min-w-0 text-right tabular-nums text-text-secondary @max-[520px]/track-browser:hidden"
        role="gridcell"
      >
        {metadata ? '—' : (row.note_count ?? '—')}
      </span>
      <span className="min-w-0 text-right tabular-nums text-text-secondary" role="gridcell">
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
        className="absolute inset-x-1.5 grid h-[46px] min-w-0 grid-cols-[36px_minmax(0,1fr)_32px_72px_76px] items-center gap-2 px-[10px] pr-3 text-[13px] text-text-tertiary opacity-70 @max-[520px]/track-browser:grid-cols-[30px_minmax(0,1fr)_32px_76px] @max-[520px]/track-browser:px-1 @max-[520px]/track-browser:pr-2 @max-[440px]/track-browser:grid-cols-[26px_minmax(0,1fr)_32px_70px] @max-[440px]/track-browser:gap-[5px] @max-[440px]/track-browser:px-0.5 @max-[440px]/track-browser:pr-1"
        role="row"
        aria-busy="true"
        aria-rowindex={index + 2}
        style={{ transform: `translateY(${start}px)` }}
      >
        <span className="min-w-0 text-center tabular-nums" role="gridcell">
          {sourceIndex + 1}
        </span>
        <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap" role="gridcell">
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

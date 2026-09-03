import type { CSSProperties } from 'react';
import type { SongRow } from '../../bridge/DesktopBridge';

interface TrackRowProps {
  index: number;
  row: SongRow;
  selected: boolean;
  start: number;
  onFocus: () => void;
  onSelect: () => void;
}

export function formatDuration(durationUs: number | null): string {
  if (durationUs === null) return '—';
  const seconds = Math.max(0, Math.round(durationUs / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

function riskLabel(value: SongRow['risk_level']): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

export function TrackRow({ index, row, selected, start, onFocus, onSelect }: TrackRowProps) {
  const style: CSSProperties = { transform: `translateY(${start}px)` };
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
        {row.note_count ?? '—'}
      </span>
      <span className={`track-cell track-cell-risk risk-${row.risk_level}`} role="gridcell">
        <span className="risk-dot" aria-hidden="true" />
        <span>{riskLabel(row.risk_level)}</span>
      </span>
      <span className="track-cell track-cell-duration" role="gridcell">
        {formatDuration(row.duration_us)}
      </span>
    </div>
  );
}

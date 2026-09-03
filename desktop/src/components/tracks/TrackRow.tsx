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

function metadataStatus(state: SongRow['metadata_state']): {
  label: string;
  value: string;
} {
  return state === 'pending'
    ? { label: 'Metadata loading', value: '…' }
    : { label: 'Metadata unavailable', value: 'Unavailable' };
}

export function TrackRow({ index, row, selected, start, onFocus, onSelect }: TrackRowProps) {
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
      <span
        className={`track-cell track-cell-risk${metadata ? ` metadata-${row.metadata_state}` : ` risk-${row.risk_level}`}`}
        role="gridcell"
      >
        {metadata ? (
          <span
            className={`track-metadata-status is-${row.metadata_state}`}
            aria-label={metadata.label}
            title={metadata.label}
          >
            {metadata.value}
          </span>
        ) : (
          <>
            <span className="risk-dot" aria-hidden="true" />
            <span>{riskLabel(row.risk_level)}</span>
          </>
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
}

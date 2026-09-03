import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { selectSelectedDetail, selectSongById } from '../../state/store';
import type { SongRow } from '../../bridge/DesktopBridge';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';

interface SongDetailsViewProps {
  useStore: DesktopStoreHook;
}

function durationLabel(durationUs: number): string {
  const seconds = Math.max(0, Math.round(durationUs / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

function metadataSummary(row: SongRow): string {
  const duration = row.duration_us === null ? '…' : durationLabel(row.duration_us);
  const notes = row.note_count === null ? '…' : `${row.note_count}`;
  return `${row.format_label} · ${duration} · ${notes} notes`;
}

export function SongDetailsView({ useStore }: SongDetailsViewProps) {
  const detail = useStore((store: DesktopStore) => selectSelectedDetail(store));
  const selectedSongId = useStore((store: DesktopStore) => store.library.selectedSongId);
  const selectedRow = useStore((store) =>
    selectSongById(store.library, store.library.selectedSongId),
  );
  const scrollRef = useScrollVisibility<HTMLElement>();

  if (!selectedSongId || !selectedRow) {
    return (
      <section
        ref={scrollRef}
        className="song-details-view scroll-surface utility-empty"
        aria-labelledby="song-details-title"
      >
        <h3 id="song-details-title">Song Details</h3>
        <p className="muted">Select a song to view details.</p>
      </section>
    );
  }

  const song = detail.value?.song_id === selectedSongId ? detail.value : null;
  const summary = metadataSummary(selectedRow);
  const isPending = detail.state === 'loading' || !song;
  const detailError = detail.state === 'fatal' ? detail.error : null;
  return (
    <section
      ref={scrollRef}
      className="song-details-view scroll-surface"
      aria-labelledby="song-details-title"
      aria-busy={detail.state === 'loading'}
      role={detail.state === 'fatal' ? 'alert' : undefined}
    >
      <div className="song-details-heading">
        <h3 id="song-details-title" title={song?.title ?? selectedRow.title}>
          {selectedRow.title}
        </h3>
        <p className="muted">{summary}</p>
      </div>

      <section
        className={`utility-risk-summary ${song ? `risk-${song.risk.level}` : 'risk-unknown'}`}
      >
        <div>
          <span className="utility-section-label">TIMING RISK</span>
          <strong>{song?.risk.headline ?? (detailError ? 'Unavailable' : '…')}</strong>
        </div>
        <span className="utility-risk-level">{song?.risk.level ?? '…'}</span>
      </section>

      <section className="utility-section" aria-live={isPending ? 'polite' : undefined}>
        <h4>Why this rating</h4>
        <ul>
          {song?.risk.reasons.length ? (
            song.risk.reasons.map((reason) => <li key={reason}>{reason}</li>)
          ) : (
            <li className={detailError ? 'inline-error' : 'muted'}>
              {detailError ?? (isPending ? '…' : 'No additional timing factors.')}
            </li>
          )}
        </ul>
      </section>

      <section className="utility-section">
        <h4>Recommendation</h4>
        <p>{song?.recommendation?.summary ?? detailError ?? '…'}</p>
        <dl className="recommendation-grid">
          <div>
            <dt>Hold</dt>
            <dd>{song?.recommendation?.recommended_hold_frames ?? '—'}</dd>
          </div>
          <div>
            <dt>Tempo</dt>
            <dd>
              {song?.recommendation?.recommended_tempo_scale
                ? `${song.recommendation.recommended_tempo_scale}×`
                : '—'}
            </dd>
          </div>
        </dl>
      </section>

      <p className="utility-note">
        Use the Player Bar to test playback without input or request playback with the selected
        settings.
      </p>
    </section>
  );
}

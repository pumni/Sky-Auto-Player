import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { selectSongById } from '../../state/store';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';

interface SongDetailsViewProps {
  useStore: DesktopStoreHook;
}

function durationLabel(durationUs: number): string {
  const seconds = Math.max(0, Math.round(durationUs / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

export function SongDetailsView({ useStore }: SongDetailsViewProps) {
  const detail = useStore((store: DesktopStore) => store.detail);
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
  const summary =
    selectedRow.metadata_state === 'ready'
      ? `${selectedRow.note_count ?? '—'} notes · ${durationLabel(selectedRow.duration_us ?? 0)}`
      : 'Preparing song metadata…';
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
          {song?.title ?? selectedRow.title}
        </h3>
        <p className="muted">
          {song
            ? `${song.format_label} · ${durationLabel(song.duration_us)} · ${song.note_count} notes`
            : summary}
        </p>
      </div>

      {song ? (
        <>
          <section className={`utility-risk-summary risk-${song.risk.level}`}>
            <div>
              <span className="utility-section-label">TIMING RISK</span>
              <strong>{song.risk.headline}</strong>
            </div>
            <span className="utility-risk-level">{song.risk.level}</span>
          </section>

          {song.risk.reasons.length > 0 && (
            <section className="utility-section">
              <h4>Why this rating</h4>
              <ul>
                {song.risk.reasons.map((reason) => (
                  <li key={reason}>{reason}</li>
                ))}
              </ul>
            </section>
          )}

          <section className="utility-section">
            <h4>Recommendation</h4>
            <p>{song.recommendation?.summary ?? 'No recommendation is available for this song.'}</p>
            {song.recommendation && (
              <dl className="recommendation-grid">
                <div>
                  <dt>Hold</dt>
                  <dd>{song.recommendation.recommended_hold_frames ?? '—'}</dd>
                </div>
                <div>
                  <dt>Tempo</dt>
                  <dd>
                    {song.recommendation.recommended_tempo_scale
                      ? `${song.recommendation.recommended_tempo_scale}×`
                      : '—'}
                  </dd>
                </div>
              </dl>
            )}
          </section>
        </>
      ) : (
        <section className="utility-section utility-detail-pending" aria-live="polite">
          <h4>Song details</h4>
          <p className={detail.state === 'fatal' ? 'inline-error' : 'muted'}>
            {detail.state === 'fatal' ? detail.error : 'Reading metadata…'}
          </p>
          <dl className="recommendation-grid" aria-hidden="true">
            <div>
              <dt>Timing risk</dt>
              <dd>—</dd>
            </div>
            <div>
              <dt>Recommendation</dt>
              <dd>—</dd>
            </div>
          </dl>
        </section>
      )}

      {song && (
        <p className="utility-note">
          Use the Player Bar to test playback without input or request playback with the selected
          settings.
        </p>
      )}
    </section>
  );
}

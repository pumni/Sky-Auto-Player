import { Music2 } from 'lucide-react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface SongInspectorProps {
  useStore: DesktopStoreHook;
}

function durationLabel(durationUs: number): string {
  const seconds = Math.max(0, Math.round(durationUs / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

export function SongInspector({ useStore }: SongInspectorProps) {
  const detail = useStore((store: DesktopStore) => store.detail);
  const selectedSongId = useStore((store: DesktopStore) => store.library.selectedSongId);

  if (!selectedSongId) {
    return (
      <section className="inspector-panel inspector-empty" aria-labelledby="inspector-title">
        <div className="empty-inspector-icon" aria-hidden="true">
          <Music2 size={26} />
        </div>
        <h2 id="inspector-title">Choose a song</h2>
        <p className="muted">
          Select a sheet from Library to inspect timing risk and playback recommendations.
        </p>
      </section>
    );
  }

  if (detail.state === 'loading') {
    return (
      <section className="inspector-panel" aria-live="polite">
        <h2>Reading metadata…</h2>
      </section>
    );
  }

  if (detail.state === 'fatal' || !detail.value) {
    return (
      <section className="inspector-panel" role="alert">
        <h2>Metadata unavailable</h2>
        <p className="inline-error">{detail.error}</p>
      </section>
    );
  }

  const song = detail.value;
  return (
    <section className="inspector-panel" aria-labelledby="inspector-title">
      <div className="inspector-identity">
        <div className="music-tile" aria-hidden="true">
          <Music2 size={28} />
        </div>
        <div>
          <h2 id="inspector-title">{song.title}</h2>
          <p className="muted">
            {song.format_label} · {durationLabel(song.duration_us)} · {song.note_count} notes
          </p>
        </div>
      </div>
      <div className={`risk-summary risk-${song.risk.level}`}>
        <div>
          <p className="eyebrow">TIMING RISK</p>
          <strong>{song.risk.headline}</strong>
        </div>
        <span className="risk-word">{song.risk.level}</span>
      </div>
      {song.risk.reasons.length > 0 && (
        <div className="inspector-section">
          <h3>Why this rating</h3>
          <ul>
            {song.risk.reasons.map((reason) => (
              <li key={reason}>{reason}</li>
            ))}
          </ul>
        </div>
      )}
      <div className="inspector-section recommendation">
        <h3>Recommendation</h3>
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
      </div>
      <p className="nonphysical-note">
        Use the Player Bar below to test playback without input or request playback with the
        selected settings.
      </p>
    </section>
  );
}

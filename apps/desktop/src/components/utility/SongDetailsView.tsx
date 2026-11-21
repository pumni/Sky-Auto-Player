import { useId, useState } from 'react';
import { MousePointer2 } from 'lucide-react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { selectSelectedDetail, selectSongById } from '../../state/store';
import type { SheetConversionMode, SheetPitch, SongRow } from '../../bridge/DesktopBridge';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import { AppSelect } from '../ui/AppSelect';

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
  const conversionMode = useStore((store) => store.library.conversionMode);
  const targetPitch = useStore((store) => store.library.targetPitch);
  const prepared = useStore((store) => store.playback.prepared);
  const playbackDefaults = useStore((store) => store.settings?.playback_defaults ?? null);
  const patchSettings = useStore((store) => store.patchSettings);
  const setConversionMode = useStore((store) => store.setSheetConversionMode);
  const setTargetPitch = useStore((store) => store.setSheetTargetPitch);
  const acknowledgeLoss = useStore((store) => store.acknowledgeSheetLoss);
  const selectedRow = useStore((store) =>
    selectSongById(store.library, store.library.selectedSongId),
  );
  const conversionModeId = useId();
  const targetPitchId = useId();
  const [applyingRecommendation, setApplyingRecommendation] = useState(false);
  const scrollRef = useScrollVisibility<HTMLElement>();

  if (!selectedSongId || !selectedRow) {
    return (
      <section
        ref={scrollRef}
        className="song-details-view scroll-surface utility-empty"
        aria-label="Song details"
        tabIndex={0}
      >
        <MousePointer2 className="utility-empty-icon" size={24} aria-hidden="true" />
        <strong>No song selected</strong>
        <p className="muted">Select a song to view details.</p>
      </section>
    );
  }

  const song = detail.value?.song_id === selectedSongId ? detail.value : null;
  const sheet = song?.sheet_analysis ?? null;
  const proposal =
    prepared?.song.song_id === selectedSongId &&
    prepared.error_code === 'conversion_confirmation_required'
      ? prepared
      : null;
  const proposalSheet = proposal?.song.sheet_analysis ?? null;
  const summary = metadataSummary(selectedRow);
  const isPending = detail.state === 'loading' || !song;
  const detailError = detail.state === 'fatal' ? detail.error : null;
  const showRiskGuidance =
    isPending ||
    detailError !== null ||
    song?.risk.level !== 'low' ||
    (song?.risk.reasons.length ?? 0) > 0;
  const recommendedHold = song?.recommendation?.recommended_hold_frames ?? null;
  const recommendedTempo = song?.recommendation?.recommended_tempo_scale ?? null;
  const hasActionableRecommendation =
    playbackDefaults !== null && (recommendedHold !== null || recommendedTempo !== null);
  const recommendationMatchesCurrent =
    hasActionableRecommendation &&
    (recommendedHold === null || playbackDefaults.hold_frames === recommendedHold) &&
    (recommendedTempo === null ||
      Math.abs(playbackDefaults.tempo_scale - recommendedTempo) < 0.001);

  const applyRecommendation = async () => {
    if (!hasActionableRecommendation || recommendationMatchesCurrent || applyingRecommendation) {
      return;
    }
    setApplyingRecommendation(true);
    try {
      await patchSettings({
        playbackDefaults: {
          ...(recommendedHold !== null ? { holdFrames: recommendedHold } : {}),
          ...(recommendedTempo !== null ? { tempoScale: recommendedTempo } : {}),
        },
      });
    } finally {
      setApplyingRecommendation(false);
    }
  };
  return (
    <section
      ref={scrollRef}
      className="song-details-view scroll-surface"
      aria-labelledby="song-details-title"
      aria-busy={detail.state === 'loading'}
      tabIndex={0}
      role={detail.state === 'fatal' ? 'alert' : undefined}
    >
      <div className="song-details-heading">
        <h3 id="song-details-title" title={song?.title ?? selectedRow.title}>
          {selectedRow.title}
        </h3>
        <p className="muted">{summary}</p>
      </div>

      {sheet && (
        <section
          className="utility-section sheet-compatibility"
          aria-labelledby="sheet-analysis-title"
        >
          <h4 id="sheet-analysis-title">Sheet compatibility</h4>
          <p className="sheet-format-label">
            {sheet.format_label}
            {sheet.schema_version === null ? '' : ` · schema ${sheet.schema_version}`}
            {' · '}
            {sheet.readiness.replaceAll('_', ' ')}
          </p>
          {sheet.error_message && (
            <p className="inline-error" role="alert">
              {sheet.error_message} Playback is blocked until the sheet is corrected or converted
              from a supported source.
            </p>
          )}

          {sheet.schema_version !== null &&
            sheet.readiness !== 'unsupported' &&
            sheet.readiness !== 'malformed' && (
              <>
                {sheet.tracks.length > 1 && (
                  <div className="sheet-choice">
                    <label htmlFor={conversionModeId}>Tracks</label>
                    <AppSelect
                      id={conversionModeId}
                      ariaLabel="Conversion mode"
                      listboxLabel="Conversion mode"
                      density="compact"
                      value={
                        conversionMode.kind === 'merge_playable'
                          ? 'merge'
                          : `track:${conversionMode.track_id}`
                      }
                      options={[
                        { value: 'merge', label: 'Merge playable tracks' },
                        ...sheet.tracks.map((track) => ({
                          value: `track:${track.track_id}`,
                          label: track.instrument ?? `Track ${track.track_id + 1}`,
                          ...(track.effective_pitch ? { hint: track.effective_pitch } : {}),
                        })),
                      ]}
                      onChange={(value) => {
                        const mode: SheetConversionMode =
                          value === 'merge'
                            ? { kind: 'merge_playable' }
                            : {
                                kind: 'single_track',
                                track_id: Number(value.slice('track:'.length)),
                              };
                        void setConversionMode(mode);
                      }}
                    />
                  </div>
                )}
                <div className="sheet-choice">
                  <label htmlFor={targetPitchId}>Target Sky Piano pitch</label>
                  <AppSelect
                    id={targetPitchId}
                    ariaLabel="Target Sky Piano pitch"
                    listboxLabel="Target Sky Piano pitch"
                    density="compact"
                    value={targetPitch ?? 'source'}
                    options={[
                      { value: 'source', label: 'Use source song pitch', hint: 'Auto' },
                      ...(
                        ['C', 'Db', 'D', 'Eb', 'E', 'F', 'Gb', 'G', 'Ab', 'A', 'Bb', 'B'] as const
                      ).map((pitch) => ({
                        value: pitch,
                        label: pitch,
                      })),
                    ]}
                    onChange={(value) =>
                      void setTargetPitch(value === 'source' ? null : (value as SheetPitch))
                    }
                  />
                </div>
                {sheet.target_pitch && (
                  <p className="sheet-pitch-warning" role="note">
                    In-game warning: use a Sky Piano tuned to {sheet.target_pitch}. The app cannot
                    detect in-game tuning;{' '}
                    {sheet.target_pitch_assumed_from_source
                      ? 'this pitch is assumed from the source song.'
                      : 'this is the target pitch you selected.'}
                  </p>
                )}
                <details className="sheet-technical-details">
                  <summary>Technical conversion counts</summary>
                  <dl className="sheet-diagnostics-grid">
                    <div>
                      <dt>Source notes</dt>
                      <dd>{sheet.source_note_count ?? '—'}</dd>
                    </div>
                    <div>
                      <dt>Source tracks</dt>
                      <dd>{sheet.source_track_count ?? '—'}</dd>
                    </div>
                    <div>
                      <dt>Output onsets</dt>
                      <dd>{sheet.output_onset_count ?? '—'}</dd>
                    </div>
                    <div>
                      <dt>Same-onset merges</dt>
                      <dd>{sheet.merged_duplicate_count ?? '—'}</dd>
                    </div>
                    <div>
                      <dt>Tempo collapses</dt>
                      <dd>{sheet.tempo_quantization_collapse_count ?? '—'}</dd>
                    </div>
                    <div>
                      <dt>Omitted track events</dt>
                      <dd>{sheet.omitted_source_event_count ?? '—'}</dd>
                    </div>
                    <div>
                      <dt>Unpitched / unresolved</dt>
                      <dd>
                        {(sheet.source_unpitched_count ?? 0) + (sheet.source_unresolved_count ?? 0)}
                      </dd>
                    </div>
                    <div>
                      <dt>Outside target range</dt>
                      <dd>{sheet.target_unplayable_count ?? '—'}</dd>
                    </div>
                    <div>
                      <dt>Impossible same-key repeats</dt>
                      <dd>{sheet.impossible_same_key_repeats ?? '—'}</dd>
                    </div>
                  </dl>
                </details>
                {(sheet.authored_sustain_lost_count ?? 0) > 0 && (
                  <p className="sheet-sustain-note">
                    {sheet.authored_sustain_lost_count} authored sustained note(s) become one-shot
                    Piano presses; source sustain is not reproduced.
                  </p>
                )}
                {sheet.loss_reasons.length > 0 && (
                  <ul className="sheet-loss-list">
                    {sheet.loss_reasons.map((reason) => (
                      <li key={reason}>{reason}</li>
                    ))}
                  </ul>
                )}
                {sheet.readiness === 'requires_confirmation' && !proposal && (
                  <p className="sheet-consent-hint">
                    Play will build an exact plan and ask you to accept these conversion losses
                    before the plan can be prepared.
                  </p>
                )}
                {proposal && proposalSheet && (
                  <div
                    className="sheet-consent-panel"
                    role="group"
                    aria-label="Conversion loss consent"
                  >
                    <p>
                      This plan uses {proposalSheet.format_label},{' '}
                      {proposalSheet.target_pitch ?? 'an unknown'} target pitch, and the selected
                      track mode. Accept the listed losses to prepare it.
                    </p>
                    <button type="button" onClick={() => void acknowledgeLoss()}>
                      Accept conversion losses and prepare
                    </button>
                  </div>
                )}
                {prepared?.song.song_id === selectedSongId &&
                  prepared.error_code !== 'conversion_confirmation_required' &&
                  prepared.song.sheet_analysis?.confirmation_received && (
                    <p className="sheet-consent-accepted">
                      Conversion losses accepted for this plan.
                    </p>
                  )}
              </>
            )}
        </section>
      )}

      <section
        className={`utility-risk-summary ${song ? `risk-${song.risk.level}` : 'risk-unknown'}`}
      >
        <div>
          <span className="utility-section-label">Timing risk</span>
          <strong>{song?.risk.headline ?? (detailError ? 'Unavailable' : '…')}</strong>
        </div>
      </section>

      {showRiskGuidance ? (
        <>
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
            {hasActionableRecommendation && (
              <div className="song-details-apply-row">
                <button
                  type="button"
                  className="button song-details-apply-button"
                  disabled={recommendationMatchesCurrent || applyingRecommendation}
                  onClick={() => void applyRecommendation()}
                >
                  {recommendationMatchesCurrent
                    ? 'Recommended settings applied'
                    : applyingRecommendation
                      ? 'Applying…'
                      : 'Apply recommendation'}
                </button>
              </div>
            )}
          </section>
        </>
      ) : (
        <details className="utility-section utility-risk-more">
          <summary>Timing rationale and settings</summary>
          <p>{song?.recommendation?.summary ?? 'Current playback settings look suitable.'}</p>
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
        </details>
      )}
    </section>
  );
}

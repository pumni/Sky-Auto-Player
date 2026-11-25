import { Check, CircleHelp, CircleX, Clock3, MousePointer2, TriangleAlert } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import type { SheetConversionMode, SheetPitch, SongRow } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { selectSelectedDetail, selectSongById } from '../../state/store';
import { Button } from '../ui/button';
import { StatusBadge } from '../ui/StatusBadge';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import {
  ScrollAreaContent,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from '../ui/scroll-area';
import { resolveInspectorReadiness } from './inspectorReadiness';

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
  const [applyingSongId, setApplyingSongId] = useState<string | null>(null);
  const [applyError, setApplyError] = useState<{ songId: string; message: string } | null>(null);
  const [prevSelectedSongId, setPrevSelectedSongId] = useState(selectedSongId);
  const mountedRef = useRef(true);
  const activeRequestIdRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    activeRequestIdRef.current += 1;
  }, [selectedSongId]);

  if (selectedSongId !== prevSelectedSongId) {
    setPrevSelectedSongId(selectedSongId);
    setApplyingSongId(null);
    setApplyError(null);
  }

  if (!selectedSongId || !selectedRow) {
    return (
      <ScrollAreaRoot className="min-h-0 min-w-0 flex-1">
        <ScrollAreaViewport
          className="size-full"
          aria-label="Song details"
          role="region"
          tabIndex={0}
        >
          <ScrollAreaContent className="grid min-h-full min-w-0 content-center justify-items-center gap-2 p-[18px] text-center">
            <MousePointer2 className="text-text-tertiary" size={24} aria-hidden="true" />
            <strong className="text-[13px] font-semibold text-text-secondary">
              No song selected
            </strong>
            <p className="m-0 max-w-[250px] text-xs leading-relaxed text-muted-foreground">
              Select a song to view details.
            </p>
          </ScrollAreaContent>
        </ScrollAreaViewport>
        <ScrollAreaScrollbar orientation="vertical">
          <ScrollAreaThumb />
        </ScrollAreaScrollbar>
      </ScrollAreaRoot>
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
  const readiness = resolveInspectorReadiness({
    pending: detail.state === 'loading' || (!song && detail.state !== 'fatal'),
    detailUnavailable: detailError !== null,
    sheetReadiness: sheet?.readiness ?? null,
    conversionConfirmed: sheet?.confirmation_received ?? false,
    riskLevel: song?.risk.level ?? null,
  });
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

  const isApplying = applyingSongId !== null && applyingSongId === selectedSongId;
  const currentError = applyError?.songId === selectedSongId ? applyError.message : null;
  const readinessTones = {
    pending: 'neutral',
    unavailable: 'neutral',
    ready: 'success',
    review: 'warning',
    blocked: 'danger',
  } as const;
  const readinessTone = readinessTones[readiness.state];
  const ReadinessIcon = {
    pending: Clock3,
    unavailable: CircleHelp,
    ready: Check,
    review: TriangleAlert,
    blocked: CircleX,
  }[readiness.state];

  const applyRecommendation = async () => {
    if (!hasActionableRecommendation || recommendationMatchesCurrent || isApplying) {
      return;
    }
    const targetSongId = selectedSongId;
    if (!targetSongId) return;
    const requestId = ++activeRequestIdRef.current;
    setApplyingSongId(targetSongId);
    setApplyError(null);
    try {
      const result = await patchSettings({
        playbackDefaults: {
          ...(recommendedHold !== null ? { holdFrames: recommendedHold } : {}),
          ...(recommendedTempo !== null ? { tempoScale: recommendedTempo } : {}),
        },
      });
      if (!mountedRef.current || activeRequestIdRef.current !== requestId) return;
      if (!result) {
        setApplyError({
          songId: targetSongId,
          message: 'Failed to apply recommendation. Please retry.',
        });
      }
    } catch (error) {
      if (!mountedRef.current || activeRequestIdRef.current !== requestId) return;
      setApplyError({
        songId: targetSongId,
        message:
          error instanceof Error ? error.message : 'Failed to apply recommendation. Please retry.',
      });
    } finally {
      if (mountedRef.current && activeRequestIdRef.current === requestId) {
        setApplyingSongId(null);
      }
    }
  };
  return (
    <ScrollAreaRoot className="min-h-0 min-w-0 flex-1">
      <ScrollAreaViewport
        className="size-full"
        aria-labelledby="song-details-title"
        data-song-details
        aria-busy={detail.state === 'loading'}
        tabIndex={0}
        role={detail.state === 'fatal' ? 'alert' : 'region'}
      >
        <ScrollAreaContent className="grid min-w-0 gap-3 px-4 pb-4 pt-3">
          <div className="grid min-w-0 gap-1 border-b border-border pb-3">
            <span className="text-[11px] font-semibold uppercase tracking-wide text-text-tertiary">
              Selected song
            </span>
            <h3
              className="m-0 mt-1 overflow-hidden text-ellipsis whitespace-nowrap text-lg font-semibold leading-6 text-foreground"
              id="song-details-title"
              title={song?.title ?? selectedRow.title}
            >
              {selectedRow.title}
            </h3>
            <p className="m-0 mt-1 text-xs text-text-secondary">{summary}</p>
          </div>

          <section
            data-slot="inspector-readiness"
            data-readiness={readiness.state}
            className="grid min-w-0 gap-1.5 border-b border-border pb-3"
            aria-labelledby="inspector-readiness-title"
            aria-busy={readiness.state === 'pending'}
            aria-live="polite"
            aria-atomic="true"
          >
            <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
              <h4
                id="inspector-readiness-title"
                className="m-0 text-sm font-semibold leading-5 text-foreground"
              >
                Readiness
              </h4>
              <StatusBadge
                tone={readinessTone}
                appearance={
                  readiness.state === 'review' || readiness.state === 'blocked' ? 'soft' : 'plain'
                }
                className="gap-1.5 px-0 [&>span:first-child]:hidden"
                data-slot="inspector-readiness-status"
                data-readiness={readiness.state}
              >
                <ReadinessIcon size={14} aria-hidden="true" />
                {readiness.label}
              </StatusBadge>
            </div>
            <p className="m-0 break-words text-xs leading-[1.45] text-text-secondary">
              {readiness.detail}
            </p>
          </section>

          {sheet && (
            <section className="grid min-w-0 gap-2" aria-labelledby="sheet-analysis-title">
              <h4
                className="mb-0 text-sm font-semibold leading-5 text-text-secondary"
                id="sheet-analysis-title"
              >
                Sheet compatibility
              </h4>
              <p className="m-0 font-semibold capitalize text-foreground">
                {sheet.format_label}
                {sheet.schema_version === null ? '' : ` · schema ${sheet.schema_version}`}
                {' · '}
                {sheet.readiness.replaceAll('_', ' ')}
              </p>
              {sheet.error_message && (
                <p
                  className="m-0 rounded-control border border-danger-border bg-danger-surface px-2 py-1.5 text-xs text-danger"
                  role="alert"
                >
                  {sheet.error_message} Playback is blocked until the sheet is corrected or
                  converted from a supported source.
                </p>
              )}

              {sheet.schema_version !== null &&
                sheet.readiness !== 'unsupported' &&
                sheet.readiness !== 'malformed' && (
                  <>
                    {sheet.tracks.length > 1 && (
                      <div className="grid min-w-0 gap-1.5">
                        <label
                          className="cursor-pointer text-xs font-medium text-text-secondary"
                          htmlFor={conversionModeId}
                        >
                          Tracks
                        </label>
                        <Select
                          value={
                            conversionMode.kind === 'merge_playable'
                              ? 'merge'
                              : `track:${conversionMode.track_id}`
                          }
                          onValueChange={(value) => {
                            if (value === null) return;
                            const mode: SheetConversionMode =
                              value === 'merge'
                                ? { kind: 'merge_playable' }
                                : {
                                    kind: 'single_track',
                                    track_id: Number(value.slice('track:'.length)),
                                  };
                            void setConversionMode(mode);
                          }}
                        >
                          <SelectTrigger
                            id={conversionModeId}
                            className="h-8"
                            aria-label="Conversion mode"
                          >
                            <SelectValue>
                              {(value: string | null) =>
                                value === 'merge'
                                  ? 'Merge playable tracks'
                                  : value?.startsWith('track:')
                                    ? (sheet.tracks.find(
                                        (track) => `track:${track.track_id}` === value,
                                      )?.instrument ??
                                      `Track ${Number(value.slice('track:'.length)) + 1}`)
                                    : 'Conversion mode'
                              }
                            </SelectValue>
                          </SelectTrigger>
                          <SelectContent className="min-w-52">
                            <SelectItem value="merge">Merge playable tracks</SelectItem>
                            {sheet.tracks.map((track) => (
                              <SelectItem key={track.track_id} value={`track:${track.track_id}`}>
                                <span>{track.instrument ?? `Track ${track.track_id + 1}`}</span>
                                {track.effective_pitch && (
                                  <span className="text-text-tertiary">
                                    {track.effective_pitch}
                                  </span>
                                )}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    )}
                    <div className="grid min-w-0 gap-1.5">
                      <label
                        className="cursor-pointer text-xs font-medium text-text-secondary"
                        htmlFor={targetPitchId}
                      >
                        Target Sky Piano pitch
                      </label>
                      <Select
                        value={targetPitch ?? 'source'}
                        onValueChange={(value) => {
                          if (value !== null) {
                            void setTargetPitch(value === 'source' ? null : (value as SheetPitch));
                          }
                        }}
                      >
                        <SelectTrigger
                          id={targetPitchId}
                          className="h-8"
                          aria-label="Target Sky Piano pitch"
                        >
                          <SelectValue>
                            {(value: string | null) =>
                              value === 'source'
                                ? 'Use source song pitch'
                                : (value ?? 'Target pitch')
                            }
                          </SelectValue>
                        </SelectTrigger>
                        <SelectContent className="min-w-52">
                          <SelectItem value="source">
                            <span>Use source song pitch</span>
                            <span className="text-text-tertiary">Auto</span>
                          </SelectItem>
                          {(
                            [
                              'C',
                              'Db',
                              'D',
                              'Eb',
                              'E',
                              'F',
                              'Gb',
                              'G',
                              'Ab',
                              'A',
                              'Bb',
                              'B',
                            ] as const
                          ).map((pitch) => (
                            <SelectItem key={pitch} value={pitch}>
                              {pitch}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    {sheet.target_pitch && (
                      <p
                        className="m-0 border-l-2 border-warning px-2 py-2 text-xs text-warning"
                        role="note"
                      >
                        In-game warning: use a Sky Piano tuned to {sheet.target_pitch}. The app
                        cannot detect in-game tuning;{' '}
                        {sheet.target_pitch_assumed_from_source
                          ? 'this pitch is assumed from the source song.'
                          : 'this is the target pitch you selected.'}
                      </p>
                    )}
                    <details className="grid min-w-0 gap-2">
                      <summary className="cursor-pointer text-xs font-semibold text-text-secondary hover:text-foreground">
                        Technical conversion counts
                      </summary>
                      <dl className="m-0 grid grid-cols-2 gap-x-3 gap-y-2">
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Source notes</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.source_note_count ?? '—'}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Source tracks</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.source_track_count ?? '—'}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Output onsets</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.output_onset_count ?? '—'}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Same-onset merges</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.merged_duplicate_count ?? '—'}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Tempo collapses</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.tempo_quantization_collapse_count ?? '—'}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Omitted track events</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.omitted_source_event_count ?? '—'}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Unpitched / unresolved</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {(sheet.source_unpitched_count ?? 0) +
                              (sheet.source_unresolved_count ?? 0)}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">Outside target range</dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.target_unplayable_count ?? '—'}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-[11px] text-text-tertiary">
                            Impossible same-key repeats
                          </dt>
                          <dd className="mb-0 mt-px text-xs font-semibold tabular-nums text-foreground">
                            {sheet.impossible_same_key_repeats ?? '—'}
                          </dd>
                        </div>
                      </dl>
                    </details>
                    {(sheet.authored_sustain_lost_count ?? 0) > 0 && (
                      <p className="m-0 border-l-2 border-warning bg-surface px-2 py-2 text-xs text-warning">
                        {sheet.authored_sustain_lost_count} authored sustained note(s) become
                        one-shot Piano presses; source sustain is not reproduced.
                      </p>
                    )}
                    {sheet.loss_reasons.length > 0 && (
                      <ul className="grid gap-1 pl-4 text-xs text-text-secondary">
                        {sheet.loss_reasons.map((reason) => (
                          <li key={reason}>{reason}</li>
                        ))}
                      </ul>
                    )}
                    {sheet.readiness === 'requires_confirmation' && !proposal && (
                      <p className="m-0 text-xs text-text-secondary">
                        Play will build an exact plan and ask you to accept these conversion losses
                        before the plan can be prepared.
                      </p>
                    )}
                    {proposal && proposalSheet && (
                      <div
                        className="grid gap-2 rounded-control border border-warning-border bg-warning-surface p-2.5"
                        role="group"
                        aria-label="Conversion loss consent"
                        data-slot="sheet-conversion-consent"
                      >
                        <p className="m-0 text-xs leading-relaxed text-warning">
                          This plan uses {proposalSheet.format_label},{' '}
                          {proposalSheet.target_pitch ?? 'an unknown'} target pitch, and the
                          selected track mode. Accept the listed losses to prepare it.
                        </p>
                        <Button
                          variant="default"
                          className="min-h-[34px] px-2.5 py-1.5 text-xs"
                          type="button"
                          onClick={() => void acknowledgeLoss()}
                        >
                          Accept conversion losses and prepare
                        </Button>
                      </div>
                    )}
                    {prepared?.song.song_id === selectedSongId &&
                      prepared.error_code !== 'conversion_confirmation_required' &&
                      prepared.song.sheet_analysis?.confirmation_received && (
                        <p
                          data-slot="sheet-consent-accepted"
                          className="m-0 text-xs text-success"
                          role="status"
                        >
                          Conversion losses accepted for this plan.
                        </p>
                      )}
                  </>
                )}
            </section>
          )}

          <section
            className="mt-0 flex min-w-0 items-center justify-between gap-3 border-b border-border py-2 data-[risk=low]:text-success data-[risk=medium]:text-warning data-[risk=high]:text-danger data-[risk=unknown]:text-muted-foreground"
            data-risk={song?.risk.level ?? 'unknown'}
          >
            <div className="min-w-0">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-text-tertiary">
                Timing risk
              </span>
              <strong className="mt-1 block text-[13px] font-semibold">
                {song?.risk.headline ?? (detailError ? 'Unavailable' : '…')}
              </strong>
            </div>
          </section>

          {showRiskGuidance ? (
            <>
              <section
                className="mt-0 border-b border-border pb-3"
                aria-live={isPending ? 'polite' : undefined}
              >
                <h4 className="mb-2 text-[13px] font-semibold text-text-secondary">
                  Why this rating
                </h4>
                <ul className="m-0 grid gap-1.5 pl-[17px] text-xs text-text-secondary">
                  {song?.risk.reasons.length ? (
                    song.risk.reasons.map((reason) => <li key={reason}>{reason}</li>)
                  ) : (
                    <li className={detailError ? 'text-danger' : 'text-muted-foreground'}>
                      {detailError ?? (isPending ? '…' : 'No additional timing factors.')}
                    </li>
                  )}
                </ul>
              </section>

              <section
                data-slot="timing-recommendation"
                className="mt-0 border-b border-border pb-3"
              >
                <h4 className="mb-2 text-[13px] font-semibold text-text-secondary">
                  Recommendation
                </h4>
                <p className="m-0 text-xs text-text-secondary">
                  {song?.recommendation?.summary ?? detailError ?? '…'}
                </p>
                <dl data-slot="recommendation-values" className="mt-3 grid grid-cols-2 gap-3">
                  <div>
                    <dt className="text-[11px] text-text-tertiary">Hold</dt>
                    <dd className="mb-0 mt-1 font-mono tabular-nums text-foreground">
                      {song?.recommendation?.recommended_hold_frames ?? '—'}
                    </dd>
                  </div>
                  <div>
                    <dt className="text-[11px] text-text-tertiary">Tempo</dt>
                    <dd className="mb-0 mt-1 font-mono tabular-nums text-foreground">
                      {song?.recommendation?.recommended_tempo_scale
                        ? `${song.recommendation.recommended_tempo_scale}×`
                        : '—'}
                    </dd>
                  </div>
                </dl>
                {hasActionableRecommendation && (
                  <div className="mt-3">
                    <Button
                      variant="default"
                      type="button"
                      className="min-h-8 w-full justify-center text-xs"
                      disabled={recommendationMatchesCurrent || isApplying}
                      onClick={() => void applyRecommendation()}
                    >
                      {recommendationMatchesCurrent
                        ? 'Recommended settings applied'
                        : isApplying
                          ? 'Applying…'
                          : currentError
                            ? 'Retry recommendation'
                            : 'Apply recommendation'}
                    </Button>
                    {currentError && (
                      <p
                        className="mt-2 rounded-control border border-danger-border bg-danger-surface px-2 py-1.5 text-xs text-danger"
                        role="alert"
                      >
                        {currentError}
                      </p>
                    )}
                  </div>
                )}
              </section>
            </>
          ) : (
            <details data-slot="timing-recommendation" className="mt-0 border-b border-border pb-3">
              <summary className="cursor-pointer text-xs font-semibold text-text-secondary hover:text-foreground">
                Timing rationale and settings
              </summary>
              <p className="mb-0 mt-2 text-xs text-text-secondary">
                {song?.recommendation?.summary ?? 'Current playback settings look suitable.'}
              </p>
              <dl className="mt-3 grid grid-cols-2 gap-3">
                <div>
                  <dt className="text-[11px] text-text-tertiary">Hold</dt>
                  <dd className="mb-0 mt-1 font-mono tabular-nums text-foreground">
                    {song?.recommendation?.recommended_hold_frames ?? '—'}
                  </dd>
                </div>
                <div>
                  <dt className="text-[11px] text-text-tertiary">Tempo</dt>
                  <dd className="mb-0 mt-1 font-mono tabular-nums text-foreground">
                    {song?.recommendation?.recommended_tempo_scale
                      ? `${song.recommendation.recommended_tempo_scale}×`
                      : '—'}
                  </dd>
                </div>
              </dl>
            </details>
          )}
        </ScrollAreaContent>
      </ScrollAreaViewport>
      <ScrollAreaScrollbar orientation="vertical">
        <ScrollAreaThumb />
      </ScrollAreaScrollbar>
    </ScrollAreaRoot>
  );
}

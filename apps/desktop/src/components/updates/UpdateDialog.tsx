import { AlertCircle, ArrowDownToLine, Check, LoaderCircle, Sparkles, X } from 'lucide-react';
import type { UpdateErrorCode, UpdateRetryAction } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../../state/store';
import { formatUpdateError } from '../../state/updateHelpers';
import { Button } from '../ui/button';
import {
  ScrollAreaContent,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from '../ui/scroll-area';
import { Dialog, DialogContent, DialogTitle } from '../ui/dialog';

function getErrorHeading(
  errorCode: UpdateErrorCode | null,
  retryAction: UpdateRetryAction,
  hasTransportError = false,
): string {
  if (hasTransportError) {
    return retryAction === 'install' ? 'Update failed' : 'Update check failed';
  }
  if (
    errorCode === 'check_failed' ||
    errorCode === 'state_persistence_failed' ||
    errorCode === 'channel_unavailable' ||
    errorCode === 'update_service_unavailable'
  ) {
    return 'Update check failed';
  }
  if (
    errorCode === 'playback_active' ||
    errorCode === 'update_busy' ||
    errorCode === 'closing' ||
    errorCode === 'stale_update' ||
    errorCode === 'update_unavailable' ||
    errorCode === 'download_failed' ||
    errorCode === 'install_failed' ||
    retryAction === 'install'
  ) {
    return 'Update failed';
  }
  return 'Update check failed';
}

interface UpdateDialogProps {
  useStore: DesktopStoreHook;
  onCloseComplete?: () => void;
}

export function UpdateDialog({ useStore, onCloseComplete }: UpdateDialogProps) {
  const update = useStore((store) => store.update);
  const close = useStore((store) => store.setUpdateDialogOpen);
  const check = useStore((store) => store.checkForUpdate);
  const handoff = useStore((store) => store.beginUpdateHandoff);
  const patchSettings = useStore((store) => store.patchSettings);

  const busy =
    update.installRequestPending || ['downloading', 'ready', 'installing'].includes(update.state);
  const isChecking = update.checkRequestPending || update.state === 'checking';
  const isError = update.state === 'error' || Boolean(update.transportError);
  const isAvailable = !isError && update.state === 'available' && Boolean(update.availableVersion);
  const isCurrent = !isError && update.state === 'current';

  const effectiveRetryAction = update.transportError
    ? (update.transportErrorAction ?? 'check')
    : update.retryAction;

  const errorText = update.transportError
    ? formatUpdateError(null, update.transportError)
    : formatUpdateError(update.errorCode, update.errorDetail);

  const hasDeterminateProgress =
    busy && typeof update.progress?.total === 'number' && update.progress.total > 0;
  const progressValue = hasDeterminateProgress
    ? Math.min(update.progress?.total ?? 0, Math.max(0, update.progress?.completed ?? 0))
    : undefined;

  return (
    <Dialog
      open={update.dialogOpen}
      onOpenChange={(open) => {
        if (!open && !busy) close(false);
      }}
      onOpenChangeComplete={(open) => {
        if (!open) onCloseComplete?.();
      }}
    >
      <DialogContent
        aria-label="Software update"
        className="flex max-h-[calc(100dvh-32px)] w-[min(540px,calc(100vw-32px))] min-h-0 min-w-0 flex-col gap-0 overflow-hidden p-5 shadow-2xl"
        showCloseButton={false}
      >
        <div
          data-slot="update-dialog-header"
          className="mb-3.5 flex flex-none items-start justify-between gap-3.5"
        >
          {isAvailable ? (
            <div className="flex min-w-0 flex-1 items-center gap-3">
              <div
                className="grid size-9 flex-none place-items-center rounded-pane border border-primary/30 bg-selection text-accent-ink"
                aria-hidden="true"
              >
                <Sparkles size={18} />
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1" role="status" aria-live="polite">
                <DialogTitle className="m-0 break-words text-xl font-semibold leading-7">
                  Version {update.availableVersion} is available
                </DialogTitle>
                <p className="m-0 text-xs leading-relaxed tabular-nums text-text-secondary">
                  You are running {update.currentVersion ?? 'the current version'} on the{' '}
                  {update.channel} channel.
                </p>
              </div>
            </div>
          ) : (
            <DialogTitle className="m-0 text-xl font-semibold leading-7">
              Software update
            </DialogTitle>
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-8 flex-none border-0 bg-transparent p-0 hover:bg-control-hover"
            type="button"
            aria-label="Close update"
            disabled={busy}
            onClick={() => close(false)}
          >
            <X size={16} aria-hidden="true" />
          </Button>
        </div>
        {isAvailable ? (
          <>
            {update.releaseNotes && (
              <ScrollAreaRoot className="min-h-0 min-w-0 flex-1 rounded-pane border border-border bg-surface">
                <ScrollAreaViewport
                  data-slot="update-notes-region"
                  aria-labelledby="update-release-notes-heading"
                  className="focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
                  role="region"
                  tabIndex={0}
                >
                  <ScrollAreaContent className="grid min-w-0 gap-2 p-3.5 text-text-secondary">
                    <h3
                      className="m-0 flex-none text-xs font-semibold leading-4 text-text-tertiary"
                      id="update-release-notes-heading"
                    >
                      What’s new
                    </h3>
                    <p
                      data-slot="update-release-notes"
                      className="m-0 min-w-0 break-words text-[13px] leading-relaxed text-text-secondary [overflow-wrap:anywhere]"
                    >
                      {update.releaseNotes}
                    </p>
                  </ScrollAreaContent>
                </ScrollAreaViewport>
                <ScrollAreaScrollbar orientation="vertical">
                  <ScrollAreaThumb />
                </ScrollAreaScrollbar>
              </ScrollAreaRoot>
            )}
            <div
              data-slot="update-dialog-actions"
              className="mt-4 flex flex-none items-center justify-between gap-2 border-t border-border pt-3.5"
            >
              <Button
                variant="ghost"
                className="text-text-tertiary hover:text-foreground"
                type="button"
                disabled={busy}
                onClick={async () => {
                  const targetVersion = update.availableVersion;
                  if (targetVersion) {
                    const result = await patchSettings({
                      updatePreferences: { skipVersion: targetVersion },
                    });
                    if (
                      result !== null &&
                      result.update_preferences.skip_version === targetVersion
                    ) {
                      close(false);
                    }
                  }
                }}
              >
                Skip this version
              </Button>
              <div className="ml-auto inline-flex flex-wrap items-center gap-2">
                <Button
                  variant="outline"
                  className="h-9"
                  type="button"
                  disabled={busy}
                  onClick={() => close(false)}
                >
                  Later
                </Button>
                <Button
                  className="h-9"
                  type="button"
                  disabled={busy}
                  onClick={() => void handoff()}
                >
                  <ArrowDownToLine size={15} aria-hidden="true" /> Update and restart
                </Button>
              </div>
            </div>
          </>
        ) : isError ? (
          <>
            <div className="mb-1 flex items-center gap-3.5">
              <div
                className="grid size-9 flex-none place-items-center rounded-pane border border-danger-border bg-danger-surface text-danger"
                aria-hidden="true"
              >
                <AlertCircle size={18} />
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1" role="status" aria-live="polite">
                <h3 className="m-0 text-xl font-semibold leading-7 text-foreground">
                  {getErrorHeading(
                    update.errorCode,
                    effectiveRetryAction,
                    Boolean(update.transportError),
                  )}
                </h3>
              </div>
            </div>
            <ScrollAreaRoot className="mt-2 min-h-0 flex-1 rounded-control border border-danger-border bg-danger-surface">
              <ScrollAreaViewport data-slot="update-error-region" role="alert">
                <ScrollAreaContent className="min-w-0 break-words p-3 text-[13px] leading-relaxed text-danger [overflow-wrap:anywhere]">
                  {errorText}
                </ScrollAreaContent>
              </ScrollAreaViewport>
              <ScrollAreaScrollbar orientation="vertical">
                <ScrollAreaThumb />
              </ScrollAreaScrollbar>
            </ScrollAreaRoot>
            <div
              data-slot="update-dialog-actions"
              className="mt-4 flex flex-none items-center justify-end gap-2 border-t border-border pt-3.5"
            >
              <Button
                variant="outline"
                className="h-9"
                type="button"
                disabled={busy}
                onClick={() => close(false)}
              >
                Close
              </Button>
              {effectiveRetryAction === 'check' && (
                <Button
                  className="h-9"
                  type="button"
                  disabled={isChecking || busy}
                  onClick={() => void check('manual')}
                >
                  Check again
                </Button>
              )}
              {effectiveRetryAction === 'install' && (
                <Button
                  className="h-9"
                  type="button"
                  disabled={busy}
                  onClick={() => void handoff()}
                >
                  Try again
                </Button>
              )}
            </div>
          </>
        ) : isChecking ? (
          <>
            <div className="mb-1 flex items-center gap-3.5">
              <div
                className="grid size-9 flex-none place-items-center rounded-pane border border-primary/30 bg-selection text-accent-ink"
                aria-hidden="true"
              >
                <LoaderCircle className="animate-spin motion-reduce:animate-none" size={18} />
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1" role="status" aria-live="polite">
                <h3 className="m-0 text-xl font-semibold leading-7 text-foreground">
                  Checking for updates…
                </h3>
                <p className="m-0 text-sm text-muted-foreground">
                  Looking for available updates on the {update.channel} channel…
                </p>
              </div>
            </div>
            <div className="mt-4 flex flex-none items-center justify-end gap-2 border-t border-border pt-3.5">
              <Button
                variant="outline"
                className="h-9"
                type="button"
                disabled={busy}
                onClick={() => close(false)}
              >
                Close
              </Button>
            </div>
          </>
        ) : isCurrent ? (
          <>
            <div className="mb-1 flex items-center gap-3.5">
              <div
                className="grid size-9 flex-none place-items-center rounded-pane border border-success-border bg-success-surface text-success"
                aria-hidden="true"
              >
                <Check size={18} />
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1" role="status" aria-live="polite">
                <h3 className="m-0 text-xl font-semibold leading-7 text-foreground">
                  You're up to date
                </h3>
                <p className="m-0 text-sm text-muted-foreground">
                  You are running version {update.currentVersion ?? 'the latest version'} on the{' '}
                  {update.channel} channel.
                </p>
              </div>
            </div>
            <div className="mt-4 flex flex-none items-center justify-end gap-2 border-t border-border pt-3.5">
              <Button
                variant="outline"
                className="h-9"
                type="button"
                disabled={busy}
                onClick={() => close(false)}
              >
                Close
              </Button>
            </div>
          </>
        ) : (
          <>
            <div className="mb-1 flex items-center gap-3.5">
              <div
                className={`grid size-9 flex-none place-items-center rounded-pane border ${update.state === 'ready' ? 'border-success-border bg-success-surface text-success' : 'border-primary/30 bg-selection text-accent-ink'}`}
                aria-hidden="true"
              >
                {busy && update.state !== 'ready' ? (
                  <LoaderCircle className="animate-spin motion-reduce:animate-none" size={18} />
                ) : (
                  <Check size={18} />
                )}
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1" role="status" aria-live="polite">
                <h3 className="m-0 text-xl font-semibold leading-7 text-foreground">
                  {update.state === 'downloading'
                    ? 'Downloading update…'
                    : update.state === 'ready'
                      ? 'Update ready to install'
                      : update.state === 'installing'
                        ? 'Installing update…'
                        : 'No update available'}
                </h3>
                <p className="m-0 text-sm text-muted-foreground">
                  {busy
                    ? `${update.progress?.message || 'The update is being applied.'}${update.progress?.total ? ` (${update.progress.completed}/${update.progress.total} bytes)` : ''}`
                    : 'The installed application is up to date.'}
                </p>
              </div>
            </div>
            {busy && (
              <div className="mt-3.5" aria-hidden="true">
                {hasDeterminateProgress ? (
                  <progress
                    className="update-progress-bar block h-1.5 w-full appearance-none overflow-hidden rounded-full border-0 bg-surface accent-primary"
                    value={progressValue}
                    max={update.progress?.total ?? 100}
                  />
                ) : (
                  <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-surface">
                    <span className="absolute inset-y-0 w-[38%] rounded-full bg-primary animate-[update-progress-slide_1.4s_ease-in-out_infinite] motion-reduce:animate-none" />
                  </div>
                )}
              </div>
            )}
            {!busy && (
              <div className="mt-4 flex flex-none items-center justify-end gap-2 border-t border-border pt-3.5">
                <Button
                  variant="outline"
                  className="h-9"
                  type="button"
                  onClick={() => close(false)}
                >
                  Close
                </Button>
              </div>
            )}
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

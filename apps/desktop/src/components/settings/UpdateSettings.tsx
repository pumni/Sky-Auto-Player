import { RefreshCw } from 'lucide-react';
import { useId } from 'react';
import type { Bootstrap, SettingsPatch } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { deriveUpdatePresentation } from '../../state/updateHelpers';
import { Button } from '../ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { SettingsCategoryHeader, SettingsSection, SettingsSwitchRow } from './SettingsLayout';

interface UpdateSettingsProps {
  bootstrap: Bootstrap;
  settings: NonNullable<DesktopStore['settings']>;
  useStore: DesktopStoreHook;
  onUpdateDialogOpen?: () => void;
}

export function UpdateSettings({
  bootstrap,
  settings,
  useStore,
  onUpdateDialogOpen,
}: UpdateSettingsProps) {
  const channelId = useId();
  const patchSettings = useStore((store) => store.patchSettings);
  const update = useStore((store) => store.update);
  const checkForUpdate = useStore((store) => store.checkForUpdate);
  const setUpdateDialogOpen = useStore((store) => store.setUpdateDialogOpen);
  const patch = (value: SettingsPatch) => void patchSettings(value);

  const {
    isChecking,
    isBusy,
    isError,
    isAvailable,
    isCurrent,
    effectiveRetryAction,
    errorText,
    busyHeading,
  } = deriveUpdatePresentation(update);

  const channelLabel =
    settings.update_preferences.channel === 'beta' ? 'Beta (unavailable)' : 'Stable';

  const openExistingDialog = () => {
    onUpdateDialogOpen?.();
    setUpdateDialogOpen(true);
  };

  const triggerManualCheck = () => {
    onUpdateDialogOpen?.();
    void checkForUpdate('manual');
  };

  return (
    <section
      data-settings-category="updates"
      data-slot="settings-category-content"
      className="grid content-start gap-4"
      aria-labelledby="update-settings-title"
    >
      <SettingsCategoryHeader
        id="update-settings-title"
        title="Updates"
        description="Check for signed desktop releases and manage automatic checks."
      />

      <div
        data-slot="update-status-group"
        className="grid min-w-0 gap-3 rounded-pane border border-border bg-card px-4 py-3.5"
      >
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <div className="grid min-w-0 gap-0.5">
            <span className="text-xs leading-[18px] text-text-tertiary">Installed version</span>
            <span className="break-words text-xl font-semibold leading-7 tabular-nums text-foreground">
              {bootstrap.app_version}
            </span>
          </div>
          <span className="text-xs leading-[18px] text-text-secondary">
            Channel: {channelLabel}
          </span>
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border pt-3">
          {isError ? (
            <p
              className="m-0 min-w-0 flex-1 break-words text-xs leading-[18px] text-danger [overflow-wrap:anywhere]"
              role="alert"
            >
              {errorText}
            </p>
          ) : (
            <p
              className="m-0 min-w-0 flex-1 break-words text-xs leading-[18px] text-text-secondary [overflow-wrap:anywhere]"
              role="status"
              aria-live="polite"
            >
              {isChecking
                ? 'Checking for updates…'
                : isBusy
                  ? busyHeading
                  : isAvailable
                    ? `Update available: ${update.availableVersion}`
                    : isCurrent
                      ? "You're up to date"
                      : 'Check for signed releases when you are ready.'}
            </p>
          )}

          <div className="flex flex-none flex-wrap items-center gap-2">
            {isError ? (
              <>
                {effectiveRetryAction === 'check' && (
                  <Button
                    variant="outline"
                    className="h-9 gap-2 whitespace-nowrap"
                    type="button"
                    disabled={isChecking || isBusy}
                    onClick={triggerManualCheck}
                  >
                    <RefreshCw size={14} aria-hidden="true" />
                    Check again
                  </Button>
                )}
                <Button
                  variant="outline"
                  className="h-9 whitespace-nowrap"
                  type="button"
                  onClick={openExistingDialog}
                >
                  View details
                </Button>
              </>
            ) : isAvailable || isBusy ? (
              <Button
                variant={isAvailable ? 'default' : 'outline'}
                className="h-9 whitespace-nowrap"
                type="button"
                onClick={openExistingDialog}
              >
                View update
              </Button>
            ) : (
              <Button
                variant="outline"
                className="h-9 gap-2 whitespace-nowrap"
                type="button"
                disabled={isChecking}
                onClick={triggerManualCheck}
              >
                <RefreshCw
                  size={14}
                  className={isChecking ? 'animate-spin motion-reduce:animate-none' : undefined}
                  aria-hidden="true"
                />
                {isChecking ? 'Checking for updates…' : 'Check for updates'}
              </Button>
            )}
          </div>
        </div>
      </div>

      <SettingsSection title="Update preferences" divider>
        <SettingsSwitchRow
          label="Check for updates automatically"
          description="Check for signed releases in the background when Sky Auto Player starts."
          checked={settings.update_preferences.auto_check}
          onChange={(autoCheck) => patch({ updatePreferences: { autoCheck } })}
        />

        <div className="grid w-full max-w-[440px] gap-1.5">
          <label
            className="cursor-pointer text-[13px] font-medium leading-5 text-foreground"
            htmlFor={channelId}
          >
            Release channel
          </label>
          <Select
            value={settings.update_preferences.channel}
            onValueChange={(nextValue) => {
              if (nextValue === 'stable' || nextValue === 'beta') {
                patch({ updatePreferences: { channel: nextValue } });
              }
            }}
          >
            <SelectTrigger id={channelId} aria-label="Release channel">
              <SelectValue>
                {(value: string | null) => (value === 'beta' ? 'Beta (unavailable)' : 'Stable')}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="stable">Stable</SelectItem>
              <SelectItem value="beta" disabled>
                Beta (unavailable)
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
      </SettingsSection>
    </section>
  );
}

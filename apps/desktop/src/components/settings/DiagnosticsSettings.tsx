import { useId } from 'react';
import type { SettingsPatch } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { SettingsCategoryHeader, SettingsSwitchRow } from './SettingsLayout';

interface DiagnosticsSettingsProps {
  settings: NonNullable<DesktopStore['settings']>;
  useStore: DesktopStoreHook;
}

export function DiagnosticsSettings({ settings, useStore }: DiagnosticsSettingsProps) {
  const noteId = useId();
  const patchSettings = useStore((store) => store.patchSettings);
  const patch = (value: SettingsPatch) => void patchSettings(value);

  return (
    <section
      data-settings-category="diagnostics"
      data-slot="settings-category-content"
      className="grid content-start gap-4"
      aria-labelledby="diagnostic-settings-title"
    >
      <SettingsCategoryHeader
        id="diagnostic-settings-title"
        title="Diagnostics"
        description="Manage saved diagnostic preferences."
      />

      <div className="grid min-w-0 gap-3 border-t border-border pt-3">
        <div className="grid divide-y divide-border">
          <div className="pb-2">
            <SettingsSwitchRow
              label="Playback telemetry"
              ariaDescribedBy={noteId}
              checked={settings.telemetry_enabled}
              onChange={(telemetryEnabled) => patch({ telemetryEnabled })}
            />
          </div>
          <div className="pt-2">
            <SettingsSwitchRow
              label="Verbose diagnostics"
              ariaDescribedBy={noteId}
              checked={settings.verbose_hud}
              onChange={(verboseHud) => patch({ verboseHud })}
            />
          </div>
        </div>
        <p id={noteId} className="m-0 text-xs leading-[18px] text-text-tertiary">
          These preferences are saved. This build has no confirmed data-sharing or additional
          diagnostics display effect.
        </p>
      </div>
    </section>
  );
}

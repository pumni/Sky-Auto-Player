import type { SettingsPatch } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { SettingsCategoryHeader, SettingsSection, SettingsSwitchRow } from './SettingsLayout';

interface DiagnosticsSettingsProps {
  settings: NonNullable<DesktopStore['settings']>;
  useStore: DesktopStoreHook;
}

export function DiagnosticsSettings({ settings, useStore }: DiagnosticsSettingsProps) {
  const patchSettings = useStore((store) => store.patchSettings);
  const patch = (value: SettingsPatch) => void patchSettings(value);

  return (
    <section
      data-settings-category="diagnostics"
      data-slot="settings-category-content"
      className="grid content-start gap-5"
      aria-labelledby="diagnostic-settings-title"
    >
      <SettingsCategoryHeader
        id="diagnostic-settings-title"
        title="Diagnostics"
        description="Manage saved diagnostic preferences."
      />

      <SettingsSection title="Diagnostics preferences" divider>
        <div className="grid divide-y divide-border">
          <div className="pb-2.5">
            <SettingsSwitchRow
              label="Playback telemetry preference"
              description="Stores your telemetry preference."
              checked={settings.telemetry_enabled}
              onChange={(telemetryEnabled) => patch({ telemetryEnabled })}
            />
          </div>
          <div className="pt-2.5">
            <SettingsSwitchRow
              label="Verbose diagnostics preference"
              description="Stores your verbose diagnostics preference."
              checked={settings.verbose_hud}
              onChange={(verboseHud) => patch({ verboseHud })}
            />
          </div>
        </div>
        <p className="m-0 text-xs leading-[18px] text-text-tertiary">
          These preferences are saved, but this build does not establish a data-sharing or
          additional display effect.
        </p>
      </SettingsSection>
    </section>
  );
}

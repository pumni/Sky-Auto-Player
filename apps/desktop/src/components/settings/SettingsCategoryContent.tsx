import type { Bootstrap } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { AboutSettings } from './AboutSettings';
import { AppearanceSettings } from './AppearanceSettings';
import { DiagnosticsSettings } from './DiagnosticsSettings';
import { DonateSettings } from './DonateSettings';
import { PlaybackSettings } from './PlaybackSettings';
import { UpdateSettings } from './UpdateSettings';

export type SettingsCategory =
  | 'playback'
  | 'appearance'
  | 'diagnostics'
  | 'updates'
  | 'donate'
  | 'about';

export const SETTINGS_CATEGORIES: Array<{ id: SettingsCategory; label: string }> = [
  { id: 'playback', label: 'Playback' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'updates', label: 'Updates' },
  { id: 'donate', label: 'Donate' },
  { id: 'about', label: 'About' },
];

interface SettingsCategoryContentProps {
  category: SettingsCategory;
  bootstrap: Bootstrap;
  settings: NonNullable<DesktopStore['settings']>;
  useStore: DesktopStoreHook;
  onUpdateDialogOpen?: () => void;
}

export function SettingsCategoryContent({
  category,
  bootstrap,
  settings,
  useStore,
  onUpdateDialogOpen,
}: SettingsCategoryContentProps) {
  switch (category) {
    case 'playback':
      return <PlaybackSettings bootstrap={bootstrap} settings={settings} useStore={useStore} />;
    case 'appearance':
      return <AppearanceSettings settings={settings} useStore={useStore} />;
    case 'diagnostics':
      return <DiagnosticsSettings settings={settings} useStore={useStore} />;
    case 'updates':
      return (
        <UpdateSettings
          bootstrap={bootstrap}
          settings={settings}
          useStore={useStore}
          {...(onUpdateDialogOpen ? { onUpdateDialogOpen } : {})}
        />
      );
    case 'donate':
      return <DonateSettings />;
    case 'about':
      return <AboutSettings bootstrap={bootstrap} />;
  }
}

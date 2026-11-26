import { SettingsSwitchRow } from './SettingsLayout';

interface AutoPlaySwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
}

export function AutoPlaySwitch({ checked, onChange }: AutoPlaySwitchProps) {
  return (
    <SettingsSwitchRow
      label="Auto Play"
      description="Automatically continue to the next song after the current song finishes successfully."
      checked={checked}
      onChange={onChange}
      dataSlot="auto-play-setting-row"
    />
  );
}

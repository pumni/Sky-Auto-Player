import { useId } from 'react';
import { Switch } from '../ui/switch';

interface AutoPlaySwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
}

export function AutoPlaySwitch({ checked, onChange }: AutoPlaySwitchProps) {
  const descriptionId = useId();
  return (
    <div
      data-slot="auto-play-setting-row"
      className="flex min-w-0 items-center justify-between gap-2 px-0 py-1 text-text-secondary"
    >
      <span className="grid min-w-0 gap-0.5">
        <strong className="text-[13px] font-semibold text-foreground">Auto Play</strong>
        <span id={descriptionId} className="text-xs leading-relaxed text-muted-foreground">
          Automatically continue to the next song after the current song finishes successfully.
        </span>
      </span>
      <div className="flex flex-none items-center gap-2">
        <span className="text-xs tabular-nums text-muted-foreground" aria-live="polite">
          {checked ? 'On' : 'Off'}
        </span>
        <Switch
          checked={checked}
          onCheckedChange={onChange}
          aria-label="Auto Play"
          aria-describedby={descriptionId}
        />
      </div>
    </div>
  );
}

import { type ReactNode, useId } from 'react';
import { Switch } from '../ui/switch';

interface SettingsCategoryHeaderProps {
  id: string;
  title: string;
  description?: string;
  icon?: ReactNode;
}

export function SettingsCategoryHeader({
  id,
  title,
  description,
  icon,
}: SettingsCategoryHeaderProps) {
  return (
    <header className="grid min-w-0 gap-1">
      <div className="flex min-w-0 items-center gap-2">
        {icon}
        <h3 id={id} className="m-0 text-lg font-semibold leading-6 tracking-tight text-foreground">
          {title}
        </h3>
      </div>
      {description && (
        <p className="m-0 text-xs leading-[18px] text-text-secondary">{description}</p>
      )}
    </header>
  );
}

interface SettingsSectionProps {
  title: string;
  divider?: boolean;
  children: ReactNode;
}

export function SettingsSection({ title, divider = false, children }: SettingsSectionProps) {
  return (
    <div className={`grid min-w-0 gap-3 ${divider ? 'border-t border-border pt-5' : ''}`}>
      <h4 className="m-0 text-sm font-semibold leading-5 text-foreground">{title}</h4>
      {children}
    </div>
  );
}

interface SettingsSwitchRowProps {
  label: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  dataSlot?: string;
}

export function SettingsSwitchRow({
  label,
  description,
  checked,
  onChange,
  dataSlot = 'settings-switch-row',
}: SettingsSwitchRowProps) {
  const descriptionId = useId();
  return (
    <div
      data-slot={dataSlot}
      className="flex min-w-0 items-center justify-between gap-3 py-1 text-text-secondary"
    >
      <div className="grid min-w-0 gap-0.5">
        <span className="text-[13px] font-medium leading-5 text-foreground">{label}</span>
        <span id={descriptionId} className="text-xs leading-[18px] text-text-secondary">
          {description}
        </span>
      </div>
      <div className="flex flex-none items-center gap-2">
        <span className="text-xs tabular-nums text-text-tertiary" aria-live="polite">
          {checked ? 'On' : 'Off'}
        </span>
        <Switch
          checked={checked}
          onCheckedChange={onChange}
          aria-label={label}
          aria-describedby={descriptionId}
        />
      </div>
    </div>
  );
}

interface AutoPlaySwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
}

export function AutoPlaySwitch({ checked, onChange }: AutoPlaySwitchProps) {
  return (
    <button
      className={`auto-play-switch${checked ? ' is-on' : ''}`}
      type="button"
      role="switch"
      aria-label="Auto Play"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
    >
      <span className="auto-play-switch-label">Auto Play</span>
      <span className="auto-play-switch-track" aria-hidden="true">
        <span />
      </span>
      <span className="auto-play-switch-state">{checked ? 'On' : 'Off'}</span>
    </button>
  );
}

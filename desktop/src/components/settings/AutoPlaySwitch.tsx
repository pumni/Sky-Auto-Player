import { useId } from 'react';

interface AutoPlaySwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
}

export function AutoPlaySwitch({ checked, onChange }: AutoPlaySwitchProps) {
  const descriptionId = useId();
  return (
    <button
      className={`auto-play-switch${checked ? ' is-on' : ''}`}
      type="button"
      role="switch"
      aria-label="Auto Play"
      aria-checked={checked}
      aria-describedby={descriptionId}
      onClick={() => onChange(!checked)}
    >
      <span className="auto-play-switch-copy">
        <span className="auto-play-switch-label">Auto Play</span>
        <span id={descriptionId} className="auto-play-switch-description">
          Automatically continue to the next song after the current song finishes successfully.
        </span>
      </span>
      <span className="auto-play-switch-control" aria-hidden="true">
        <span className="auto-play-switch-track">
          <span />
        </span>
        <span className="auto-play-switch-state">{checked ? 'On' : 'Off'}</span>
      </span>
    </button>
  );
}

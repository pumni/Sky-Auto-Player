import type { Bootstrap, SettingsPatch, ThemeId } from '../../bridge/DesktopBridge';
import type { DesktopStore as StoreState, DesktopStoreHook } from '../../state/store';

interface SettingsPanelProps {
  bootstrap: Bootstrap;
  useStore: DesktopStoreHook;
}

const themes: Array<{ id: ThemeId; label: string }> = [
  { id: 'aurora', label: 'Aurora' },
  { id: 'minimalist', label: 'Minimalist' },
  { id: 'slate', label: 'Slate' },
  { id: 'cyberpunk', label: 'Cyberpunk' },
  { id: 'classic', label: 'Classic' },
];

export function SettingsPanel({ bootstrap, useStore }: SettingsPanelProps) {
  const settings = useStore((store: StoreState) => store.settings);
  const open = useStore((store: StoreState) => store.settingsOpen);
  const setOpen = useStore((store: StoreState) => store.setSettingsOpen);
  const patchSettings = useStore((store: StoreState) => store.patchSettings);
  if (!open || !settings) return null;

  const patch = (value: SettingsPatch) => void patchSettings(value);
  const defaults = settings.playback_defaults;
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) setOpen(false);
      }}
    >
      <section
        className="settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
      >
        <div className="dialog-heading">
          <div>
            <p className="eyebrow">PREFERENCES</p>
            <h2 id="settings-title">Settings</h2>
          </div>
          <button
            className="icon-button"
            type="button"
            onClick={() => setOpen(false)}
            aria-label="Close settings"
          >
            ×
          </button>
        </div>
        <div className="settings-section">
          <h3>Playback defaults</h3>
          <div className="settings-grid">
            <label>
              Hold
              <select
                value={defaults.hold_frames}
                onChange={(event) =>
                  patch({ playbackDefaults: { holdFrames: Number(event.target.value) } })
                }
              >
                {bootstrap.option_sets.hold_frames.map((value) => (
                  <option key={value} value={value}>
                    {value} frames
                  </option>
                ))}
              </select>
            </label>
            <label>
              Tempo
              <select
                value={defaults.tempo_scale}
                onChange={(event) =>
                  patch({ playbackDefaults: { tempoScale: Number(event.target.value) } })
                }
              >
                {bootstrap.option_sets.tempo_scales.map((value) => (
                  <option key={value} value={value}>
                    {value}×
                  </option>
                ))}
              </select>
            </label>
            <label>
              FPS
              <select
                value={defaults.fps}
                onChange={(event) =>
                  patch({ playbackDefaults: { fps: Number(event.target.value) } })
                }
              >
                {bootstrap.option_sets.fps.map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </div>
        <div className="settings-section">
          <h3>Interface</h3>
          <label>
            Theme
            <select
              value={settings.theme}
              onChange={(event) => patch({ theme: event.target.value as ThemeId })}
            >
              {themes.map((theme) => (
                <option key={theme.id} value={theme.id}>
                  {theme.label}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="settings-section">
          <h3>Telemetry and diagnostics</h3>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={settings.telemetry_enabled}
              onChange={(event) => patch({ telemetryEnabled: event.target.checked })}
            />{' '}
            Allow anonymous telemetry
          </label>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={settings.verbose_hud}
              onChange={(event) => patch({ verboseHud: event.target.checked })}
            />{' '}
            Verbose fallback HUD
          </label>
        </div>
        <p className="settings-note">Settings are validated and persisted by the local Core.</p>
      </section>
    </div>
  );
}

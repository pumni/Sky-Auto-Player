import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { Bootstrap, SettingsPatch, ThemeId } from '../../bridge/DesktopBridge';
import type { DesktopStore as StoreState, DesktopStoreHook } from '../../state/store';

interface SettingsPanelProps {
  bootstrap: Bootstrap;
  useStore: DesktopStoreHook;
}

type SettingsCategory = 'playback' | 'appearance' | 'diagnostics' | 'updates' | 'advanced';

const categories: Array<{ id: SettingsCategory; label: string }> = [
  { id: 'playback', label: 'Playback' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'updates', label: 'Updates' },
  { id: 'advanced', label: 'Advanced' },
];

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
  const setCalibrationOpen = useStore((store: StoreState) => store.setCalibrationOpen);
  const update = useStore((store: StoreState) => store.update);
  const checkForUpdate = useStore((store: StoreState) => store.checkForUpdate);
  const dialogRef = useRef<HTMLElement>(null);
  const wasOpen = useRef(false);
  const [category, setCategory] = useState<SettingsCategory>('playback');

  useEffect(() => {
    if (open) {
      wasOpen.current = true;
      setCategory('playback');
      dialogRef.current?.focus();
    } else if (wasOpen.current) {
      wasOpen.current = false;
      document.querySelector<HTMLElement>('[aria-label="Open settings"]')?.focus();
    }
  }, [open]);

  if (!open || !settings) return null;

  const patch = (value: SettingsPatch) => void patchSettings(value);
  const defaults = settings.playback_defaults;
  return (
    <ModalOverlay
      className="modal-backdrop"
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) setOpen(false);
      }}
    >
      <Modal>
        <Dialog ref={dialogRef} aria-label="Settings" className="settings-dialog">
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
              <X size={16} aria-hidden="true" />
            </button>
          </div>

          <div className="settings-layout">
            <nav className="settings-nav" aria-label="Settings categories">
              {categories.map((item) => (
                <button
                  key={item.id}
                  className={`settings-nav-item${category === item.id ? ' is-active' : ''}`}
                  type="button"
                  aria-current={category === item.id ? 'page' : undefined}
                  onClick={() => setCategory(item.id)}
                >
                  {item.label}
                </button>
              ))}
            </nav>

            <div className="settings-content">
              {category === 'playback' && (
                <section className="settings-section" aria-labelledby="playback-settings-title">
                  <h3 id="playback-settings-title">Playback defaults</h3>
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
                  <p className="settings-note">
                    These defaults are used when preparing a new playback session.
                  </p>
                </section>
              )}

              {category === 'appearance' && (
                <section className="settings-section" aria-labelledby="appearance-settings-title">
                  <h3 id="appearance-settings-title">Appearance</h3>
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
                  <p className="settings-note">Choose a quiet, high-contrast workspace palette.</p>
                </section>
              )}

              {category === 'diagnostics' && (
                <section className="settings-section" aria-labelledby="diagnostic-settings-title">
                  <h3 id="diagnostic-settings-title">Telemetry and diagnostics</h3>
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={settings.telemetry_enabled}
                      onChange={(event) => patch({ telemetryEnabled: event.target.checked })}
                    />
                    Allow anonymous telemetry
                  </label>
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={settings.verbose_hud}
                      onChange={(event) => patch({ verboseHud: event.target.checked })}
                    />
                    Verbose playback HUD
                  </label>
                </section>
              )}

              {category === 'updates' && (
                <section className="settings-section" aria-labelledby="update-settings-title">
                  <h3 id="update-settings-title">Updates</h3>
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={settings.update_preferences.auto_check}
                      onChange={(event) =>
                        patch({ updatePreferences: { autoCheck: event.target.checked } })
                      }
                    />
                    Check for updates automatically
                  </label>
                  <label>
                    Channel
                    <select
                      value={settings.update_preferences.channel}
                      onChange={(event) =>
                        patch({
                          updatePreferences: { channel: event.target.value as 'stable' | 'beta' },
                        })
                      }
                    >
                      <option value="stable">Stable</option>
                      <option value="beta">Beta</option>
                    </select>
                  </label>
                  <label>
                    Skip version
                    <input
                      value={settings.update_preferences.skip_version}
                      placeholder="Optional version"
                      onChange={(event) =>
                        patch({ updatePreferences: { skipVersion: event.target.value } })
                      }
                    />
                  </label>
                  <button className="button" type="button" onClick={() => void checkForUpdate()}>
                    {update.state === 'checking' ? 'Checking…' : 'Check for updates'}
                  </button>
                </section>
              )}

              {category === 'advanced' && (
                <section className="settings-section" aria-labelledby="advanced-settings-title">
                  <h3 id="advanced-settings-title">Advanced</h3>
                  <p className="settings-note">
                    Native timing, input and security policy remain owned by the local Rust runtime.
                  </p>
                  <div className="settings-subsection">
                    <h3>Native timing</h3>
                    <p className="settings-note">
                      Run a safe, testable timing calibration for this machine.
                    </p>
                    <button
                      className="button"
                      type="button"
                      onClick={() => {
                        setOpen(false);
                        setCalibrationOpen(true);
                      }}
                    >
                      Open calibration
                    </button>
                  </div>
                </section>
              )}
            </div>
          </div>
          <p className="settings-note settings-footer-note">
            Settings are validated and persisted by the native runtime.
          </p>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

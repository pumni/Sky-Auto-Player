import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { X } from 'lucide-react';
import { useEffect, useRef, useState, type RefObject } from 'react';
import type { Bootstrap, SettingsPatch, ThemeId } from '../../bridge/DesktopBridge';
import type { DesktopStore as StoreState, DesktopStoreHook } from '../../state/store';
import { TimingMarginControl } from './TimingMarginControl';

interface SettingsPanelProps {
  bootstrap: Bootstrap;
  settingsTriggerRef?: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

type SettingsCategory =
  'playback' | 'appearance' | 'diagnostics' | 'updates' | 'advanced' | 'about';

const categories: Array<{ id: SettingsCategory; label: string }> = [
  { id: 'playback', label: 'Playback' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'updates', label: 'Updates' },
  { id: 'advanced', label: 'Advanced' },
  { id: 'about', label: 'About' },
];

const themes: Array<{ id: ThemeId; label: string }> = [
  { id: 'aurora', label: 'Aurora' },
  { id: 'minimalist', label: 'Minimalist' },
  { id: 'slate', label: 'Slate' },
  { id: 'cyberpunk', label: 'Cyberpunk' },
  { id: 'classic', label: 'Classic' },
];

export function SettingsPanel({ bootstrap, settingsTriggerRef, useStore }: SettingsPanelProps) {
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
      dialogRef.current?.focus();
    } else if (wasOpen.current) {
      wasOpen.current = false;
      window.setTimeout(() => settingsTriggerRef?.current?.focus(), 0);
    }
  }, [open, settingsTriggerRef]);

  if (!open || !settings) return null;

  const patch = (value: SettingsPatch) => void patchSettings(value);
  const defaults = settings.playback_defaults;
  const frameUs = Math.ceil(1_000_000 / defaults.fps);
  const frameBaseHoldUs = Math.ceil(defaults.hold_frames * frameUs);
  const minHoldUs = frameBaseHoldUs + defaults.timing_margin_us;
  const minReleaseGapUs = frameUs + defaults.timing_margin_us;
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
                      Base Hold
                      <select
                        value={defaults.hold_frames}
                        onChange={(event) =>
                          patch({ playbackDefaults: { holdFrames: Number(event.target.value) } })
                        }
                      >
                        {bootstrap.option_sets.hold_frames.map((value) => (
                          <option key={value} value={value}>
                            {value} {value === 1 ? 'frame' : 'frames'}
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
                  <TimingMarginControl
                    value={defaults.timing_margin_us}
                    options={bootstrap.option_sets}
                    recommendation={settings.timing_margin_recommendation}
                    onChange={(value) =>
                      patchSettings({ playbackDefaults: { timingMarginUs: value } }).then(
                        (authoritative) =>
                          authoritative?.playback_defaults.timing_margin_us ?? null,
                      )
                    }
                  />
                  <dl className="settings-timing-summary">
                    <div>
                      <dt>1 frame</dt>
                      <dd>{(frameUs / 1_000).toFixed(3)} ms</dd>
                    </div>
                    <div>
                      <dt>Base hold</dt>
                      <dd>{(frameBaseHoldUs / 1_000).toFixed(3)} ms</dd>
                    </div>
                    <div>
                      <dt>Target hold</dt>
                      <dd>{(minHoldUs / 1_000).toFixed(3)} ms</dd>
                    </div>
                    <div>
                      <dt>Release gap</dt>
                      <dd>{(minReleaseGapUs / 1_000).toFixed(3)} ms</dd>
                    </div>
                    <div>
                      <dt>Down late cutoff</dt>
                      <dd>500 µs</dd>
                    </div>
                  </dl>
                  <p className="settings-note">
                    Playback changes apply when preparing the next session. An active session keeps
                    its frozen timing values.
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

              {category === 'about' && (
                <section className="settings-section" aria-labelledby="about-settings-title">
                  <h3 id="about-settings-title">About Sky Auto Player</h3>
                  <p className="about-intro">
                    A focused Windows workbench for preparing and playing Sky music sheets.
                  </p>
                  <dl className="about-details">
                    <div>
                      <dt>Version</dt>
                      <dd>{bootstrap.app_version}</dd>
                    </div>
                    <div>
                      <dt>Native version</dt>
                      <dd>{bootstrap.native_build.native_version}</dd>
                    </div>
                    <div>
                      <dt>Build commit</dt>
                      <dd>{bootstrap.native_build.native_build_commit}</dd>
                    </div>
                    <div>
                      <dt>Native ABI</dt>
                      <dd>{bootstrap.native_build.native_abi}</dd>
                    </div>
                    <div>
                      <dt>Rust</dt>
                      <dd>{bootstrap.native_build.rustc_version}</dd>
                    </div>
                    <div>
                      <dt>Win32 backend</dt>
                      <dd>{bootstrap.native_build.win32_backend ? 'Enabled' : 'Disabled'}</dd>
                    </div>
                  </dl>
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

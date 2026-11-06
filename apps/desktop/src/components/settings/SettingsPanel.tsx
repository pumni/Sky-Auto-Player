import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { X } from 'lucide-react';
import { useEffect, useRef, useState, type RefObject } from 'react';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import type { DesktopStore as StoreState, DesktopStoreHook } from '../../state/store';
import { SETTINGS_CATEGORIES, SettingsCategoryContent } from './SettingsCategoryContent';
import type { SettingsCategory } from './SettingsCategoryContent';

interface SettingsPanelProps {
  bootstrap: Bootstrap;
  settingsTriggerRef?: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

export function SettingsPanel({ bootstrap, settingsTriggerRef, useStore }: SettingsPanelProps) {
  const settings = useStore((store: StoreState) => store.settings);
  const settingsError = useStore((store: StoreState) => store.settingsError);
  const open = useStore((store: StoreState) => store.settingsOpen);
  const setOpen = useStore((store: StoreState) => store.setSettingsOpen);
  const dialogRef = useRef<HTMLElement>(null);
  const contentRef = useScrollVisibility<HTMLDivElement>(open);
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
              className="icon-button icon-button-boxed"
              type="button"
              onClick={() => setOpen(false)}
              aria-label="Close settings"
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>

          <div className="settings-layout">
            <nav className="settings-nav" aria-label="Settings categories">
              {SETTINGS_CATEGORIES.map((item) => (
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

            <div ref={contentRef} className="settings-content scroll-surface">
              <SettingsCategoryContent
                category={category}
                bootstrap={bootstrap}
                settings={settings}
                useStore={useStore}
              />
            </div>
          </div>
          {settingsError !== null && (
            <p className="inline-error settings-save-error" role="alert">
              Could not save settings. {settingsError}
            </p>
          )}
          <p className="settings-note settings-footer-note">
            Settings are validated and persisted by the native runtime.
          </p>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

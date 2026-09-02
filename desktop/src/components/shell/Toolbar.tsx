import { Download, ListRestart, Search, Settings2 } from 'lucide-react';
import { Button } from 'react-aria-components';
import { useEffect, useState } from 'react';
import brandMark32Url from '../../assets/brand/app-icon-32.png';
import brandMark40Url from '../../assets/brand/app-icon-40.png';
import brandMark48Url from '../../assets/brand/app-icon-48.png';
import brandMark64Url from '../../assets/brand/app-icon-64.png';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface ToolbarProps {
  bootstrap: Bootstrap;
  useStore: DesktopStoreHook;
}

export function Toolbar({ bootstrap, useStore }: ToolbarProps) {
  const query = useStore((store: DesktopStore) => store.library.query);
  const search = useStore((store: DesktopStore) => store.search);
  const reload = useStore((store: DesktopStore) => store.reloadLibrary);
  const setSettingsOpen = useStore((store: DesktopStore) => store.setSettingsOpen);
  const update = useStore((store: DesktopStore) => store.update);
  const setUpdateDialogOpen = useStore((store: DesktopStore) => store.setUpdateDialogOpen);
  const [draft, setDraft] = useState(query);

  useEffect(() => setDraft(query), [query]);
  useEffect(() => {
    const timeout = window.setTimeout(() => {
      if (draft !== query) void search(draft);
    }, 120);
    return () => window.clearTimeout(timeout);
  }, [draft, query, search]);

  return (
    <header className="toolbar">
      <div className="identity" aria-label="Sky Auto Player">
        <img
          className="identity-mark"
          src={brandMark32Url}
          srcSet={`${brandMark32Url} 1x, ${brandMark40Url} 1.25x, ${brandMark48Url} 1.5x, ${brandMark64Url} 2x`}
          sizes="32px"
          alt=""
          width="32"
          height="32"
          draggable={false}
        />
        <strong>Sky Auto Player</strong>
      </div>
      <label className="search-field">
        <Search size={16} aria-hidden="true" />
        <span className="visually-hidden">Search songs</span>
        <input
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder="Search library…"
          type="search"
          spellCheck={false}
          aria-label="Search songs"
        />
        <kbd>/</kbd>
      </label>
      <div className="toolbar-actions">
        <span className="version-label">v{bootstrap.app_version}</span>
        {update.state === 'available' && (
          <Button
            className="update-indicator"
            aria-label={`Open update ${update.availableVersion}`}
            onPress={() => setUpdateDialogOpen(true)}
          >
            <Download size={14} aria-hidden="true" />
            <span>Update</span>
          </Button>
        )}
        <Button className="icon-button" aria-label="Reload songs" onPress={() => void reload()}>
          <ListRestart size={17} aria-hidden="true" />
        </Button>
        <Button
          className="icon-button"
          aria-label="Open settings"
          onPress={() => setSettingsOpen(true)}
        >
          <Settings2 size={17} aria-hidden="true" />
        </Button>
      </div>
    </header>
  );
}

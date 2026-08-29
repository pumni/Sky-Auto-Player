import { ListRestart, Settings2, Search } from 'lucide-react';
import { Button } from 'react-aria-components';
import { useEffect, useState } from 'react';
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
        <span className="identity-mark" aria-hidden="true">
          ♪
        </span>
        <span>
          <strong>Sky Auto Player</strong>
          <small>Library</small>
        </span>
      </div>
      <label className="search-field">
        <Search size={16} aria-hidden="true" />
        <span className="visually-hidden">Search songs</span>
        <input
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder="Search songs…"
          type="search"
          spellCheck={false}
          aria-label="Search songs"
        />
        <kbd>/</kbd>
      </label>
      <div className="toolbar-actions">
        <span className="version-label">v{bootstrap.app_version}</span>
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

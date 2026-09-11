import { Search } from 'lucide-react';
import { useEffect, useState, type RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface GlobalSearchProps {
  inputRef?: RefObject<HTMLInputElement | null>;
  useStore: DesktopStoreHook;
}

export function GlobalSearch({ inputRef, useStore }: GlobalSearchProps) {
  const query = useStore((store: DesktopStore) => store.library.query);
  const playlistAddMode = useStore((store: DesktopStore) => store.library.playlistAddMode);
  return (
    <SearchInput
      key={`${query}:${playlistAddMode?.playlistId ?? 'library'}`}
      query={query}
      {...(inputRef ? { inputRef } : {})}
      useStore={useStore}
      playlistAddMode={playlistAddMode !== null}
    />
  );
}

interface SearchInputProps extends GlobalSearchProps {
  query: string;
  playlistAddMode: boolean;
}

function SearchInput({ inputRef, query, useStore, playlistAddMode }: SearchInputProps) {
  const search = useStore((store) => store.search);
  const [draft, setDraft] = useState(query);

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      if (draft !== query) void search(draft);
    }, 120);
    return () => window.clearTimeout(timeout);
  }, [draft, query, search]);

  return (
    <label className="global-search" data-tauri-drag-region="false">
      <Search size={16} aria-hidden="true" />
      <span className="visually-hidden">
        {playlistAddMode ? 'Search All Songs' : 'Search library'}
      </span>
      <input
        ref={inputRef}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        placeholder={playlistAddMode ? 'Search All Songs…' : 'Search library…'}
        type="search"
        spellCheck={false}
        aria-label={playlistAddMode ? 'Search All Songs' : 'Search library'}
        data-tauri-drag-region="false"
      />
      <kbd aria-hidden="true">/</kbd>
    </label>
  );
}

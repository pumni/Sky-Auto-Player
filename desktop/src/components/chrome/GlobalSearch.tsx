import { Search } from 'lucide-react';
import { useEffect, useRef, useState, type RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface GlobalSearchProps {
  inputRef?: RefObject<HTMLInputElement | null>;
  useStore: DesktopStoreHook;
}

export function GlobalSearch({ inputRef, useStore }: GlobalSearchProps) {
  const query = useStore((store: DesktopStore) => store.library.query);
  const source = useStore((store: DesktopStore) => store.library.source);
  const playlistAddMode = useStore((store: DesktopStore) => store.library.playlistAddMode);
  const playlistName = useStore((store: DesktopStore) => {
    const playlistId =
      store.library.playlistAddMode?.playlistId ??
      (store.library.source.kind === 'playlist' ? store.library.source.id : null);
    return playlistId
      ? (store.libraryNavigation.playlistsById.get(playlistId)?.name ?? 'playlist')
      : null;
  });
  const searchLabel = playlistAddMode
    ? `Search All Songs to add to ${playlistName ?? 'playlist'}`
    : source.kind === 'playlist'
      ? `Search ${playlistName ?? 'playlist'}`
      : source.id === 'liked'
        ? 'Search Liked Songs'
        : source.id === 'builtin'
          ? 'Search Built-in Songs'
          : 'Search All Songs';
  return (
    <SearchInput
      key={playlistAddMode?.playlistId ?? 'library'}
      query={query}
      {...(inputRef ? { inputRef } : {})}
      useStore={useStore}
      searchLabel={searchLabel}
      placeholder={playlistAddMode ? 'Search All Songs…' : `${searchLabel}…`}
    />
  );
}

interface SearchInputProps extends GlobalSearchProps {
  query: string;
  searchLabel: string;
  placeholder: string;
}

function SearchInput({ inputRef, query, useStore, searchLabel, placeholder }: SearchInputProps) {
  const search = useStore((store) => store.search);
  const [draft, setDraft] = useState(query);
  const submittedQueryRef = useRef<string | null>(null);

  useEffect(() => {
    if (submittedQueryRef.current === query) {
      submittedQueryRef.current = null;
      return;
    }
    setDraft(query);
  }, [query]);

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      if (draft !== query) {
        submittedQueryRef.current = draft;
        void search(draft);
      }
    }, 120);
    return () => window.clearTimeout(timeout);
  }, [draft, query, search]);

  return (
    <label className="global-search" data-tauri-drag-region="false">
      <Search size={16} aria-hidden="true" />
      <span className="visually-hidden">{searchLabel}</span>
      <input
        ref={inputRef}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        placeholder={placeholder}
        type="search"
        spellCheck={false}
        aria-label={searchLabel}
        data-tauri-drag-region="false"
      />
      <kbd aria-hidden="true">/</kbd>
    </label>
  );
}

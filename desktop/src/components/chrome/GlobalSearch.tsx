import { Search } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface GlobalSearchProps {
  useStore: DesktopStoreHook;
}

export function GlobalSearch({ useStore }: GlobalSearchProps) {
  const query = useStore((store: DesktopStore) => store.library.query);
  return <SearchInput key={query} query={query} useStore={useStore} />;
}

interface SearchInputProps extends GlobalSearchProps {
  query: string;
}

function SearchInput({ query, useStore }: SearchInputProps) {
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
      <span className="visually-hidden">Search library</span>
      <input
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        placeholder="Search library…"
        type="search"
        spellCheck={false}
        aria-label="Search library"
        data-tauri-drag-region="false"
      />
      <kbd aria-hidden="true">/</kbd>
    </label>
  );
}

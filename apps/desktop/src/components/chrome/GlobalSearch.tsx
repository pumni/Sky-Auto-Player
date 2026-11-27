import { Search, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import { Input } from '../ui/input';

function searchScopeKey(
  source: DesktopStore['library']['source'],
  playlistAddMode: DesktopStore['library']['playlistAddMode'],
) {
  return playlistAddMode
    ? `add:${playlistAddMode.playlistId}`
    : source.kind === 'playlist'
      ? `playlist:${source.id}`
      : `smart:${source.id}`;
}

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
    ? `Search Built-in Songs to add to ${playlistName ?? 'playlist'}`
    : source.kind === 'playlist'
      ? `Search ${playlistName ?? 'playlist'}`
      : source.id === 'liked'
        ? 'Search Liked Songs'
        : source.id === 'builtin'
          ? 'Search Built-in Songs'
          : 'Search Library';
  const scopeText = playlistAddMode
    ? 'Built-in'
    : source.kind === 'playlist'
      ? (playlistName ?? 'Playlist')
      : source.id === 'liked'
        ? 'Liked'
        : source.id === 'builtin'
          ? 'Built-in'
          : 'Library';
  const scopeKey = searchScopeKey(source, playlistAddMode);
  return (
    <SearchInput
      key={playlistAddMode?.playlistId ?? 'library'}
      query={query}
      scopeKey={scopeKey}
      {...(inputRef ? { inputRef } : {})}
      useStore={useStore}
      searchLabel={searchLabel}
      scopeText={scopeText}
      placeholder={playlistAddMode ? 'Search songs…' : 'Search current source…'}
    />
  );
}

interface SearchInputProps extends GlobalSearchProps {
  query: string;
  scopeKey: string;
  searchLabel: string;
  scopeText: string;
  placeholder: string;
}

interface SearchDraftState {
  baseQuery: string;
  draft: string;
  scopeKey: string;
}

function SearchInput({
  inputRef,
  query,
  scopeKey,
  useStore,
  searchLabel,
  scopeText,
  placeholder,
}: SearchInputProps) {
  const [draftState, setDraftState] = useState<SearchDraftState | null>(null);
  const inputElementRef = useRef<HTMLInputElement | null>(null);
  const debounceTimerRef = useRef<number | null>(null);

  const setInputElement = useCallback(
    (element: HTMLInputElement | null) => {
      inputElementRef.current = element;
      if (inputRef) inputRef.current = element;
    },
    [inputRef],
  );

  const cancelQueuedSearch = useCallback(() => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    // The store publishes query/scope changes synchronously before awaiting the bridge.
    // Cancel local typing on any new committed intent, including external navigation.
    const unsubscribe = useStore.subscribe((state, previous) => {
      const next = state.library;
      const before = previous.library;
      if (
        next.query !== before.query ||
        searchScopeKey(next.source, next.playlistAddMode) !==
          searchScopeKey(before.source, before.playlistAddMode)
      ) {
        cancelQueuedSearch();
        setDraftState(null);
      }
    });
    return () => {
      unsubscribe();
      cancelQueuedSearch();
    };
  }, [useStore, cancelQueuedSearch]);

  const inputValue =
    draftState?.scopeKey === scopeKey && draftState.baseQuery === query
      ? draftState.draft
      : query;

  const issueSearch = (nextQuery: string) => {
    // Only the store owns committed query and request-generation fencing.
    // Promise completion must never mutate the input draft.
    void useStore.getState().search(nextQuery).catch((error: unknown) => {
      if (import.meta.env.DEV) {
        console.error('Search request failed:', error);
      }
    });
  };

  const handleChange = (nextDraft: string) => {
    cancelQueuedSearch();
    const current = useStore.getState().library;
    const baseQuery = current.query;
    const baseScope = searchScopeKey(current.source, current.playlistAddMode);
    if (nextDraft === baseQuery) {
      setDraftState(null);
      return;
    }
    setDraftState({ baseQuery, draft: nextDraft, scopeKey: baseScope });
    debounceTimerRef.current = window.setTimeout(() => {
      debounceTimerRef.current = null;
      const latest = useStore.getState().library;
      if (
        latest.query !== baseQuery ||
        searchScopeKey(latest.source, latest.playlistAddMode) !== baseScope
      ) {
        setDraftState(null);
        return;
      }
      issueSearch(nextDraft);
      setDraftState(null);
    }, 120);
  };

  const handleClear = () => {
    cancelQueuedSearch();
    // An in-flight nonempty search already committed its query synchronously.
    // Dispatching the empty intent fences its late result through the store.
    if (useStore.getState().library.query !== '') {
      issueSearch('');
    }
    setDraftState(null);
    inputElementRef.current?.focus();
  };

  return (
    <div
      role="search"
      data-slot="global-search"
      className="flex h-8 w-full min-w-0 items-center gap-2 rounded-control border border-border bg-surface px-[9px] text-muted-foreground hover:border-overlay-border focus-within:border-overlay-border focus-within:outline focus-within:outline-2 focus-within:outline-offset-1 focus-within:outline-ring"
      data-tauri-drag-region="false"
    >
      <Search size={16} className="shrink-0 text-text-secondary" aria-hidden="true" />
      <span
        className="max-w-[72px] shrink-0 overflow-hidden text-ellipsis whitespace-nowrap border-r border-border pr-2 text-[11px] text-text-tertiary"
        aria-hidden="true"
      >
        {scopeText}
      </span>
      <Input
        ref={setInputElement}
        className="h-7 min-w-0 flex-1 border-0 bg-transparent px-0 py-0 text-[13px] text-foreground shadow-none placeholder:text-text-tertiary outline-none focus-visible:border-0 focus-visible:ring-0"
        value={inputValue}
        onChange={(event) => handleChange(event.target.value)}
        placeholder={placeholder}
        type="search"
        spellCheck={false}
        aria-label={searchLabel}
        data-tauri-drag-region="false"
      />
      {inputValue.length > 0 && (
        <Button
          variant="ghost"
          size="icon-xs"
          className="size-6 shrink-0 rounded-control p-0 text-text-secondary hover:bg-control-hover hover:text-foreground"
          type="button"
          aria-label="Clear search"
          title="Clear search"
          data-tauri-drag-region="false"
          onMouseDown={(event) => event.preventDefault()}
          onClick={handleClear}
        >
          <X size={14} aria-hidden="true" />
        </Button>
      )}
      <kbd
        className="rounded border border-border px-1 text-xs leading-[18px] text-muted-foreground max-[1100px]:hidden"
        aria-hidden="true"
      >
        /
      </kbd>
    </div>
  );
}

import { Search, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import { Input } from '../ui/input';

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
  const scopeKey = playlistAddMode
    ? `add:${playlistAddMode.playlistId}`
    : source.kind === 'playlist'
      ? `playlist:${source.id}`
      : `smart:${source.id}`;
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
  revision: number;
  scopeKey: string;
  ownedQueries: readonly string[];
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
  const search = useStore((store) => store.search);
  const [draftState, setDraftState] = useState<SearchDraftState | null>(null);
  const draftStateRef = useRef<SearchDraftState | null>(null);
  const inputElementRef = useRef<HTMLInputElement | null>(null);
  const debounceTimerRef = useRef<number | null>(null);
  const draftRevisionRef = useRef(0);
  const searchRequestIdRef = useRef(0);
  const pendingSearchesRef = useRef(new Map<number, string>());
  const scopeKeyRef = useRef(scopeKey);
  const observedQueryRef = useRef(query);
  const expectedOwnQueryChangesRef = useRef(new Map<string, number>());
  const mountedRef = useRef(true);

  const commitDraftState = (nextState: SearchDraftState | null) => {
    draftStateRef.current = nextState;
    setDraftState(nextState);
  };

  const updateDraftState = (
    update: (previous: SearchDraftState | null) => SearchDraftState | null,
  ) => {
    commitDraftState(update(draftStateRef.current));
  };

  const setInputElement = useCallback(
    (element: HTMLInputElement | null) => {
      inputElementRef.current = element;
      if (inputRef) inputRef.current = element;
    },
    [inputRef],
  );

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (debounceTimerRef.current !== null) {
        window.clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (scopeKeyRef.current === scopeKey) return;
    scopeKeyRef.current = scopeKey;
    draftRevisionRef.current += 1;
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    draftStateRef.current = null;
    setDraftState(null);
    pendingSearchesRef.current.clear();
    expectedOwnQueryChangesRef.current.clear();
  }, [scopeKey]);

  useEffect(() => {
    const previousQuery = observedQueryRef.current;
    observedQueryRef.current = query;
    if (previousQuery === query || scopeKeyRef.current !== scopeKey) return;
    const expectedChanges = expectedOwnQueryChangesRef.current;
    const expectedCount = expectedChanges.get(query) ?? 0;
    if (expectedCount > 0) {
      if (expectedCount === 1) expectedChanges.delete(query);
      else expectedChanges.set(query, expectedCount - 1);
      return;
    }
    const current = draftStateRef.current;
    if (!current) return;
    draftRevisionRef.current += 1;
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    draftStateRef.current = null;
    setDraftState(null);
    expectedChanges.clear();
  }, [query, scopeKey]);

  const isOwnDraft =
    draftState !== null &&
    draftState.scopeKey === scopeKey &&
    (draftState.baseQuery === query || draftState.ownedQueries.includes(query));
  const inputValue = isOwnDraft ? draftState.draft : query;

  const issueSearch = (nextQuery: string, revision: number) => {
    const requestId = ++searchRequestIdRef.current;
    updateDraftState((previous) =>
      previous && previous.revision === revision
        ? {
            ...previous,
            ownedQueries: previous.ownedQueries.includes(nextQuery)
              ? previous.ownedQueries
              : [...previous.ownedQueries, nextQuery],
          }
        : previous,
    );
    pendingSearchesRef.current.set(requestId, nextQuery);
    const expectedChanges = expectedOwnQueryChangesRef.current;
    expectedChanges.set(nextQuery, (expectedChanges.get(nextQuery) ?? 0) + 1);

    void search(nextQuery)
      .catch((error: unknown) => {
        if (import.meta.env.DEV) {
          console.error('Search request failed:', error);
        }
      })
      .finally(() => {
        if (!mountedRef.current) return;
        pendingSearchesRef.current.delete(requestId);
        updateDraftState((previous) => {
          if (!previous) return null;
          if (previous.revision === revision && previous.draft === nextQuery) {
            expectedOwnQueryChangesRef.current.clear();
            return null;
          }
          return previous;
        });
      });
  };

  const cancelQueuedSearch = () => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
  };

  const getOwnedQueries = (currentQuery: string) => {
    if (scopeKeyRef.current !== scopeKey) return [];
    const current = draftStateRef.current;
    return current &&
      (current.baseQuery === currentQuery || current.ownedQueries.includes(currentQuery))
      ? current.ownedQueries
      : [];
  };

  const handleChange = (nextDraft: string) => {
    cancelQueuedSearch();
    const currentQuery = useStore.getState().library.query;
    const ownedQueries = getOwnedQueries(currentQuery);
    const revision = ++draftRevisionRef.current;
    commitDraftState({
      baseQuery: currentQuery,
      draft: nextDraft,
      revision,
      scopeKey,
      ownedQueries,
    });

    const pendingQueries = [...pendingSearchesRef.current.values()];
    const alreadySearchingForDraft = pendingQueries.includes(nextDraft);
    const hasOtherPendingSearch = pendingQueries.some((requestQuery) => requestQuery !== nextDraft);
    if (nextDraft === currentQuery && (!hasOtherPendingSearch || alreadySearchingForDraft)) {
      commitDraftState(null);
      expectedOwnQueryChangesRef.current.clear();
      return;
    }

    debounceTimerRef.current = window.setTimeout(() => {
      debounceTimerRef.current = null;
      if (draftRevisionRef.current !== revision) return;
      issueSearch(nextDraft, revision);
    }, 120);
  };

  const handleClear = () => {
    cancelQueuedSearch();
    const currentQuery = useStore.getState().library.query;
    const ownedQueries = getOwnedQueries(currentQuery);
    const revision = ++draftRevisionRef.current;
    inputElementRef.current?.focus();

    const hasOlderPendingSearch = [...pendingSearchesRef.current.values()].some(
      (requestQuery) => requestQuery !== '',
    );
    if (currentQuery === '' && !hasOlderPendingSearch) {
      commitDraftState(null);
      expectedOwnQueryChangesRef.current.clear();
      return;
    }

    commitDraftState({ baseQuery: currentQuery, draft: '', revision, scopeKey, ownedQueries });
    issueSearch('', revision);
  };

  return (
    <div
      role="search"
      data-slot="global-search"
      className="flex h-8 w-full min-w-0 items-center gap-2 rounded-control border border-chrome-input-border bg-surface px-[9px] text-muted-foreground focus-within:border-ring focus-within:outline focus-within:outline-2 focus-within:outline-offset-1 focus-within:outline-ring"
      data-tauri-drag-region="false"
    >
      <Search size={16} className="shrink-0" aria-hidden="true" />
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

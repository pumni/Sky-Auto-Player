import { Search, X } from 'lucide-react';
import { useEffect, useRef, useState, type RefObject } from 'react';
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
  return (
    <SearchInput
      key={playlistAddMode?.playlistId ?? 'library'}
      query={query}
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
  searchLabel: string;
  scopeText: string;
  placeholder: string;
}

interface SearchDraftState {
  baseQuery: string;
  draft: string;
  inFlight: readonly string[];
}

function SearchInput({
  inputRef,
  query,
  useStore,
  searchLabel,
  scopeText,
  placeholder,
}: SearchInputProps) {
  const search = useStore((store) => store.search);
  const [draftState, setDraftState] = useState<SearchDraftState | null>(null);
  const debounceTimerRef = useRef<number | null>(null);
  const mountedRef = useRef(true);

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

  const isOwnDraft =
    draftState !== null && (draftState.baseQuery === query || draftState.inFlight.includes(query));
  const inputValue = isOwnDraft ? draftState.draft : query;

  const handleChange = (nextDraft: string) => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }

    setDraftState((previous) => {
      const keepInFlight =
        previous !== null && (previous.baseQuery === query || previous.inFlight.includes(query))
          ? previous.inFlight
          : [];
      return {
        baseQuery: query,
        draft: nextDraft,
        inFlight: keepInFlight,
      };
    });

    if (nextDraft === query) return;

    debounceTimerRef.current = window.setTimeout(() => {
      debounceTimerRef.current = null;
      setDraftState((previous) =>
        previous ? { ...previous, inFlight: [...previous.inFlight, nextDraft] } : null,
      );
      void search(nextDraft)
        .catch((error: unknown) => {
          if (import.meta.env.DEV) {
            console.error('Search request failed:', error);
          }
        })
        .finally(() => {
          if (!mountedRef.current) return;
          setDraftState((previous) => {
            if (!previous) return null;
            if (previous.draft === nextDraft) return null;
            const remaining = previous.inFlight.filter((item) => item !== nextDraft);
            return {
              ...previous,
              baseQuery: nextDraft,
              inFlight: remaining,
            };
          });
        });
    }, 120);
  };

  return (
    <div
      role="search"
      data-slot="global-search"
      className="flex h-8 w-full min-w-0 items-center gap-2 rounded-control border border-border bg-card px-[9px] text-muted-foreground focus-within:border-ring focus-within:outline focus-within:outline-2 focus-within:outline-offset-1 focus-within:outline-ring"
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
        ref={inputRef}
        className="h-7 min-w-0 flex-1 border-0 bg-transparent px-0 py-0 text-[13px] text-foreground shadow-none placeholder:text-text-tertiary outline-none focus-visible:border-0 focus-visible:ring-0 dark:bg-transparent"
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
          onClick={() => {
            handleChange('');
            inputRef?.current?.focus();
          }}
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

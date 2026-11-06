import { recordStartupTelemetry } from '../bridge/startupTelemetry';
import type { DesktopStore, LibraryState } from './types';
import type { LibrarySource } from '../bridge/DesktopBridge';
import type { LibraryPlaybackCoordinator } from './libraryPlaybackCoordinator';

const MAX_CATALOG_RECONCILIATION_ATTEMPTS = 3;

export interface LibraryCatalogContext {
  get: () => DesktopStore;
  playback: LibraryPlaybackCoordinator;
  nextDetailRequestToken: () => number;
  nextBuiltinPageIntentId: (library: LibraryState) => number;
  clearPendingBuiltinFocus: () => void;
  invalidateStaleGenerations: (generation: number) => void;
  membershipRevisionFor: (source: LibrarySource) => number;
}

export interface LibraryCatalogActions {
  syncCatalogGeneration: (generation: number) => Promise<void>;
  reconcileCatalog: () => Promise<void>;
}

export function createLibraryCatalogActions(context: LibraryCatalogContext): LibraryCatalogActions {
  const {
    get,
    playback,
    nextDetailRequestToken,
    nextBuiltinPageIntentId,
    clearPendingBuiltinFocus,
    invalidateStaleGenerations,
    membershipRevisionFor,
  } = context;
  let catalogReconciliationTail: Promise<void> = Promise.resolve();
  let catalogReadyRecorded = false;

  const syncCatalogGeneration = async (generation: number): Promise<void> => {
    const current = get().library;
    if (generation <= current.generation) return;
    const pageIntentId = nextBuiltinPageIntentId(current);
    clearPendingBuiltinFocus();
    invalidateStaleGenerations(generation);
    nextDetailRequestToken();
    playback.applyCatalogGeneration({
      generation,
      membershipRevision: membershipRevisionFor(current.searchSource),
      pageIntentId,
    });
    void get().loadLibraryNavigation();
    await get().search();
  };

  const reconcileCatalog = (): Promise<void> => {
    catalogReconciliationTail = catalogReconciliationTail.then(async () => {
      if (get().bootstrapState !== 'ready' || get().library.error !== null) return;
      for (let attempt = 0; attempt < MAX_CATALOG_RECONCILIATION_ATTEMPTS; attempt += 1) {
        const requestedGeneration = get().library.generation;
        await get().search();
        const afterSearch = get().library;
        if (afterSearch.error !== null) return;
        if (afterSearch.loading || afterSearch.generation !== requestedGeneration) continue;

        const settled = get().library;
        if (settled.error !== null || settled.loading || settled.generation !== requestedGeneration)
          continue;
        if (!catalogReadyRecorded) {
          catalogReadyRecorded = true;
          recordStartupTelemetry('react.catalog_ready');
        }
        return;
      }
      // A catalog.changed event queues another bounded reconciliation attempt.
    });
    return catalogReconciliationTail;
  };

  return { syncCatalogGeneration, reconcileCatalog };
}

import { useRef, useState } from 'react';
import {
  loadWorkbenchPreferences,
  normalizeWorkbenchPreferences,
  updateWorkbenchPreferences,
} from '../../state/workbenchPreferences';
import type {
  WorkbenchPreferencePatch,
  WorkbenchPreferencesV5,
} from '../../state/workbenchPreferences';

export function useWorkbenchPreferences() {
  const [layout, setLayout] = useState<WorkbenchPreferencesV5>(() => loadWorkbenchPreferences());
  // Keep queued event updates composable between renders without side effects in the updater.
  const pendingLayoutRef = useRef(layout);

  const update = (patch: WorkbenchPreferencePatch, persist = false) => {
    const next = normalizeWorkbenchPreferences({ ...pendingLayoutRef.current, ...patch });
    const replace = (candidate: WorkbenchPreferencesV5) => {
      const previous = pendingLayoutRef.current;
      if (
        previous.navigatorPreference === candidate.navigatorPreference &&
        previous.expandedNavigatorWidth === candidate.expandedNavigatorWidth &&
        previous.utilityWidth === candidate.utilityWidth &&
        previous.utilityOpen === candidate.utilityOpen
      )
        return;
      pendingLayoutRef.current = candidate;
      setLayout(candidate);
    };
    replace(next);
    if (persist) {
      const persisted = updateWorkbenchPreferences({
        navigatorPreference: next.navigatorPreference,
        expandedNavigatorWidth: next.expandedNavigatorWidth,
        utilityWidth: next.utilityWidth,
      });
      replace(persisted);
    }
  };

  return { layout, update };
}

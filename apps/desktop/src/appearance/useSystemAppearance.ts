import { useEffect, useState } from 'react';
import { isTauriRuntime } from '../platform/runtime';
import { readBrowserSystemMode, subscribeSystemAppearance } from '../platform/systemAppearance';
import type { ColorMode } from './resolveAppearance';
import type { SystemAppearanceSubscription } from '../platform/systemAppearance';

let sharedMode: ColorMode | null = null;
let sharedReady = false;
let activeSubscription: SystemAppearanceSubscription | null = null;
const subscribers = new Set<(state: { mode: ColorMode; ready: boolean }) => void>();

export function useSystemAppearance(): { mode: ColorMode; ready: boolean } {
  const [state, setState] = useState<{ mode: ColorMode; ready: boolean }>(() => ({
    mode: sharedMode ?? readBrowserSystemMode(),
    ready: sharedReady || !isTauriRuntime(),
  }));

  useEffect(() => {
    subscribers.add(setState);

    if (!activeSubscription) {
      sharedReady = !isTauriRuntime();
      sharedMode = readBrowserSystemMode();
      activeSubscription = subscribeSystemAppearance((nextMode) => {
        sharedMode = nextMode;
        for (const sub of subscribers) {
          sub({ mode: nextMode, ready: sharedReady });
        }
      });
      void activeSubscription.ready.then(() => {
        sharedReady = true;
        for (const sub of subscribers) {
          sub({ mode: sharedMode ?? 'dark', ready: true });
        }
      });
    }

    return () => {
      subscribers.delete(setState);
      if (subscribers.size === 0) {
        activeSubscription?.unsubscribe();
        activeSubscription = null;
        sharedMode = null;
        sharedReady = false;
      }
    };
  }, []);

  return state;
}

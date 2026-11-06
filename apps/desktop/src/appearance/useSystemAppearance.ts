import { useEffect, useState } from 'react';
import { isTauriRuntime } from '../platform/runtime';
import { readBrowserSystemMode, subscribeSystemAppearance } from '../platform/systemAppearance';
import type { ColorMode } from './resolveAppearance';

export function useSystemAppearance(): { mode: ColorMode; ready: boolean } {
  const [mode, setMode] = useState<ColorMode>(readBrowserSystemMode);
  const [ready, setReady] = useState(() => !isTauriRuntime());

  useEffect(() => {
    let active = true;
    const subscription = subscribeSystemAppearance(setMode);
    void subscription.ready.then(() => {
      if (active) setReady(true);
    });
    return () => {
      active = false;
      subscription.unsubscribe();
    };
  }, []);

  return { mode, ready };
}

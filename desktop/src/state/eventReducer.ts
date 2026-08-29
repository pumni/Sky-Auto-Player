import type { UiEvent } from '../bridge/DesktopBridge';

export interface EventState {
  catalogGeneration: number;
  catalogTotal: number;
  fatal: string | null;
}

export const initialEventState: EventState = {
  catalogGeneration: 0,
  catalogTotal: 0,
  fatal: null,
};

export function reduceEvent(state: EventState, event: UiEvent): EventState {
  if (event.name === 'catalog.changed') {
    return {
      ...state,
      catalogGeneration: event.payload.generation,
      catalogTotal: event.payload.total,
    };
  }
  if (event.name === 'core.fatal') {
    return { ...state, fatal: `${event.payload.code}: ${event.payload.message}` };
  }
  return state;
}

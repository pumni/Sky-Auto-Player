// The native event hub retains at most 500 events; this window also covers a
// full terminal-event backlog while keeping process-lifetime state bounded.
export const MAX_RETIRED_SESSION_IDS = 512;

export type RetiredSessionOutcome = 'finished' | 'failed';

export function rememberRetiredSession(
  sessions: Map<string, RetiredSessionOutcome>,
  sessionId: string,
  outcome: RetiredSessionOutcome,
): void {
  sessions.delete(sessionId);
  sessions.set(sessionId, outcome);
  while (sessions.size > MAX_RETIRED_SESSION_IDS) {
    const oldest = sessions.keys().next().value;
    if (oldest === undefined) break;
    sessions.delete(oldest);
  }
}

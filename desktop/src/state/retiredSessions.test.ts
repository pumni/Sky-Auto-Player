import { describe, expect, it } from 'vitest';
import { MAX_RETIRED_SESSION_IDS, rememberRetiredSession } from './retiredSessions';

describe('retired session history', () => {
  it('keeps a bounded insertion-ordered window large enough for the native event history', () => {
    const sessions = new Map<string, 'finished' | 'failed'>();
    for (let index = 0; index < MAX_RETIRED_SESSION_IDS + 40; index += 1) {
      rememberRetiredSession(sessions, `session-${index}`, 'finished');
    }

    expect(sessions.size).toBe(MAX_RETIRED_SESSION_IDS);
    expect(sessions.has('session-0')).toBe(false);
    expect(sessions.has('session-39')).toBe(false);
    expect(sessions.get('session-40')).toBe('finished');
    expect(sessions.get(`session-${MAX_RETIRED_SESSION_IDS + 39}`)).toBe('finished');
  });
});

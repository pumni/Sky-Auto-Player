import type { DiagnosticsSnapshot } from './DesktopBridge';

type MissReasonCounters = Pick<
  DiagnosticsSnapshot,
  | 'missed_down_boundaries'
  | 'missed_down_keys'
  | 'missed_unobserved_backlog_boundaries'
  | 'missed_physical_window_boundaries'
  | 'final_sender_window_expirations'
>;

export function assertMissReasonAccounting(snapshot: MissReasonCounters): void {
  const accounted =
    snapshot.missed_unobserved_backlog_boundaries +
    snapshot.missed_physical_window_boundaries +
    snapshot.final_sender_window_expirations;
  if (snapshot.missed_down_boundaries !== accounted) {
    throw new Error(
      `miss reason accounting drift: total=${snapshot.missed_down_boundaries}, ` +
        `backlog=${snapshot.missed_unobserved_backlog_boundaries}, ` +
        `physical=${snapshot.missed_physical_window_boundaries}, ` +
        `sender=${snapshot.final_sender_window_expirations}`,
    );
  }
  if (snapshot.missed_down_keys < snapshot.missed_down_boundaries) {
    throw new Error(
      `missed Down key accounting drift: keys=${snapshot.missed_down_keys}, ` +
        `boundaries=${snapshot.missed_down_boundaries}`,
    );
  }
}

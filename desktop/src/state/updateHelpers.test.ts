import { describe, expect, it } from 'vitest';
import { formatUpdateError, shouldAutoCheck } from './updateHelpers';

describe('updateHelpers shouldAutoCheck', () => {
  it('returns false when auto_check is disabled', () => {
    expect(
      shouldAutoCheck({
        auto_check: false,
        last_check_ts: 0,
        last_error_ts: 0,
        check_interval_s: 86400,
      }),
    ).toBe(false);
  });

  it('checks on fresh install when last_check_ts is 0 and last_error_ts is 0', () => {
    const nowSec = 1_700_000_000;
    expect(
      shouldAutoCheck(
        {
          auto_check: true,
          last_check_ts: 0,
          last_error_ts: 0,
          check_interval_s: 86400,
        },
        nowSec,
      ),
    ).toBe(true);
  });

  it('throttles check if within check_interval_s of last successful check', () => {
    const lastCheck = 1_700_000_000;
    const nowSec = lastCheck + 3600; // 1 hour later, interval is 24 hours
    expect(
      shouldAutoCheck(
        {
          auto_check: true,
          last_check_ts: lastCheck,
          last_error_ts: 0,
          check_interval_s: 86400,
        },
        nowSec,
      ),
    ).toBe(false);
  });

  it('allows check when check_interval_s has elapsed since last successful check', () => {
    const lastCheck = 1_700_000_000;
    const nowSec = lastCheck + 86400; // exactly 24 hours later
    expect(
      shouldAutoCheck(
        {
          auto_check: true,
          last_check_ts: lastCheck,
          last_error_ts: 0,
          check_interval_s: 86400,
        },
        nowSec,
      ),
    ).toBe(true);
  });

  it('throttles check if within 300s backoff after an error', () => {
    const lastCheck = 1_700_000_000;
    const lastError = 1_700_050_000;
    const nowSec = lastError + 120; // 2 minutes later
    expect(
      shouldAutoCheck(
        {
          auto_check: true,
          last_check_ts: lastCheck,
          last_error_ts: lastError,
          check_interval_s: 86400,
        },
        nowSec,
      ),
    ).toBe(false);
  });

  it('retries after 300s backoff elapses following an error', () => {
    const lastCheck = 1_700_000_000;
    const lastError = 1_700_050_000;
    const nowSec = lastError + 300; // 5 minutes later
    expect(
      shouldAutoCheck(
        {
          auto_check: true,
          last_check_ts: lastCheck,
          last_error_ts: lastError,
          check_interval_s: 86400,
        },
        nowSec,
      ),
    ).toBe(true);
  });
});

describe('updateHelpers formatUpdateError', () => {
  it('translates playback_active into user-facing copy', () => {
    const err = 'playback_active: update installation cannot run during physical playback';
    expect(formatUpdateError(err)).toBe(
      'Updates cannot be installed while music playback is active. Stop playback and try again.',
    );
  });

  it('translates calibration_active into user-facing copy', () => {
    const err = 'calibration_active: update installation cannot run during calibration';
    expect(formatUpdateError(err)).toBe(
      'Updates cannot be installed while timing calibration is running. Complete or cancel calibration and try again.',
    );
  });

  it('translates update_busy into user-facing copy', () => {
    const err = 'update_busy: another update installation is already active';
    expect(formatUpdateError(err)).toBe('An update installation is already in progress.');
  });

  it('translates closing into user-facing copy', () => {
    const err = 'closing: desktop application is closing';
    expect(formatUpdateError(err)).toBe('The application is closing.');
  });

  it('translates network errors into user-facing copy', () => {
    const err = new Error('Failed to fetch from github release API: network timeout');
    expect(formatUpdateError(err)).toBe(
      'Could not check for updates. Check your network connection and try again.',
    );
  });

  it('handles null/undefined error gracefully', () => {
    expect(formatUpdateError(null)).toBe(
      'The update service is temporarily unavailable. Please try again later.',
    );
  });
});

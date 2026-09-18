import { describe, expect, it } from 'vitest';
import { formatUpdateError, stripTechnicalPrefixes } from './updateHelpers';

describe('updateHelpers stripTechnicalPrefixes', () => {
  it('strips error: and technical identifier prefixes', () => {
    expect(stripTechnicalPrefixes('error: invalid_params: unexpected token')).toBe(
      'unexpected token',
    );
    expect(stripTechnicalPrefixes('error: check_failed: network timeout')).toBe('network timeout');
  });

  it('leaves clean strings unchanged', () => {
    expect(stripTechnicalPrefixes('Connection lost')).toBe('Connection lost');
  });
});

describe('updateHelpers formatUpdateError', () => {
  it('translates playback_active into user-facing copy', () => {
    expect(formatUpdateError('playback_active')).toBe(
      'Updates cannot be installed while music playback is active. Stop playback and try again.',
    );
    expect(
      formatUpdateError(
        'playback_active',
        'playback_active: update installation cannot run during physical playback',
      ),
    ).toBe(
      'Updates cannot be installed while music playback is active. Stop playback and try again.',
    );
  });

  it('translates calibration_active into user-facing copy', () => {
    expect(formatUpdateError('calibration_active')).toBe(
      'Updates cannot be installed while timing calibration is running. Complete or cancel calibration and try again.',
    );
  });

  it('translates update_busy into user-facing copy', () => {
    expect(formatUpdateError('update_busy')).toBe('An update installation is already in progress.');
  });

  it('translates closing into user-facing copy', () => {
    expect(formatUpdateError('closing')).toBe('The application is closing.');
  });

  it('translates channel_unavailable into user-facing copy', () => {
    expect(formatUpdateError('channel_unavailable')).toBe(
      'The selected update channel is not available.',
    );
  });

  it('translates update_service_unavailable into user-facing copy', () => {
    expect(formatUpdateError('update_service_unavailable')).toBe(
      'The update service is not available. Please try again later.',
    );
  });

  it('translates stale_update into user-facing copy', () => {
    expect(formatUpdateError('stale_update')).toBe('The requested update is no longer available.');
  });

  it('translates update_unavailable into user-facing copy', () => {
    expect(formatUpdateError('update_unavailable')).toBe(
      'No update is available for this version.',
    );
  });

  it('translates state_persistence_failed into user-facing copy', () => {
    expect(formatUpdateError('state_persistence_failed')).toBe('Failed to save update state.');
  });

  it('translates check_failed with and without detail to stable copy', () => {
    expect(formatUpdateError('check_failed')).toBe(
      'Could not check for updates. Check your network connection and try again.',
    );
    expect(formatUpdateError('check_failed', 'error: fetch_timeout: request timed out')).toBe(
      'Could not check for updates. Check your network connection and try again.',
    );
  });

  it('translates download_failed with and without detail to stable copy', () => {
    expect(formatUpdateError('download_failed')).toBe(
      'Failed to download the update. Please try again later.',
    );
    expect(formatUpdateError('download_failed', 'error: io_error: disk full')).toBe(
      'Failed to download the update. Please try again later.',
    );
  });

  it('translates install_failed with and without detail to stable copy', () => {
    expect(formatUpdateError('install_failed')).toBe(
      'Failed to install the update. Please try again later.',
    );
    expect(formatUpdateError('install_failed', 'error: execution_error: elevated abort')).toBe(
      'Failed to install the update. Please try again later.',
    );
  });

  it('translates unknown with and without detail to stable copy', () => {
    expect(formatUpdateError('unknown')).toBe('An unexpected error occurred while updating.');
    expect(formatUpdateError('unknown', 'error: custom_error: something broke')).toBe(
      'An unexpected error occurred while updating.',
    );
  });

  it('translates raw error strings with technical prefix fallback', () => {
    const raw = 'error: network_failure: Failed to fetch from github release API';
    expect(formatUpdateError(raw)).toBe('Failed to fetch from github release API');
  });

  it('handles null/undefined error gracefully', () => {
    expect(formatUpdateError(null)).toBe(
      'The update service is temporarily unavailable. Please try again later.',
    );
    expect(formatUpdateError(undefined, null)).toBe(
      'The update service is temporarily unavailable. Please try again later.',
    );
  });
});

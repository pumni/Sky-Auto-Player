import { describe, expect, it } from 'vitest';
import { getMockUpdateFixtureOptions } from './mockUpdateFixtures';

const finalNoteLine = 'FINAL RELEASE NOTE LINE — Hoàn tất cập nhật.';

describe('browser mock update fixtures', () => {
  it.each([
    { name: 'none', length: 0 },
    { name: 'short', length: 57 },
    { name: 'medium', length: 1_024 },
    { name: 'long', length: 4_096 },
    { name: 'maximum', length: 16_384 },
  ])('creates the $name release-note fixture', ({ name, length }) => {
    const fixture = getMockUpdateFixtureOptions(`?mockReleaseNotes=${name}&other=ignored`);

    expect(fixture.updateSnapshot).toMatchObject({
      state: 'available',
      current_version: '4.0.0-alpha.1-mock',
      available_version: '4.0.0-alpha.2-mock',
      channel: 'stable',
    });
    expect(fixture.updateSnapshot?.release_notes?.length ?? 0).toBe(length);
    if (length > 1_000) {
      expect(fixture.updateSnapshot?.release_notes).toContain('Cải thiện độ ổn định');
      expect(fixture.updateSnapshot?.release_notes).toContain('x'.repeat(240));
      expect(fixture.updateSnapshot?.release_notes).toContain(finalNoteLine);
      expect(fixture.updateSnapshot?.release_notes).toContain('<script>unsafe()</script>');
    }
  });

  it('creates the 4,096-character long-error fixture', () => {
    const fixture = getMockUpdateFixtureOptions('?mockUpdateState=error-long');

    expect(fixture.updateSnapshot).toBeUndefined();
    expect(fixture.updateCheckTransportError).toHaveLength(4_096);
    expect(fixture.updateCheckTransportError).toContain(finalNoteLine);
  });

  it.each(['', '?mockReleaseNotes=unsupported', '?mockUpdateState=current'])(
    'ignores unrelated or unsupported update fixture query %s',
    (search) => {
      expect(getMockUpdateFixtureOptions(search)).toEqual({});
    },
  );
});

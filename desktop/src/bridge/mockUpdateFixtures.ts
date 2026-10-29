import type { UpdateSnapshotPayload } from './DesktopBridge';

export interface MockUpdateFixtureOptions {
  updateCheckTransportError?: string;
  updateSnapshot?: Partial<UpdateSnapshotPayload>;
}

type ReleaseNoteFixture = 'none' | 'short' | 'medium' | 'long' | 'maximum';

function makeMockReleaseNotes(fixture: Exclude<ReleaseNoteFixture, 'none' | 'short'>): string;
function makeMockReleaseNotes(fixture: 'none' | 'short'): string | null;
function makeMockReleaseNotes(fixture: ReleaseNoteFixture): string | null;
function makeMockReleaseNotes(fixture: ReleaseNoteFixture): string | null {
  if (fixture === 'none') return null;
  if (fixture === 'short') return 'Bản cập nhật cải thiện độ ổn định và trải nghiệm sử dụng.';

  const lengths: Record<Exclude<ReleaseNoteFixture, 'none' | 'short'>, number> = {
    medium: 1_024,
    long: 4_096,
    maximum: 16_384,
  };
  const length = lengths[fixture];
  const finalLine = 'FINAL RELEASE NOTE LINE — Hoàn tất cập nhật.';
  const seed = [
    'Cải thiện độ ổn định, khả năng truy cập và thời gian khởi động.',
    `Unbroken text: ${'x'.repeat(240)}`,
    'Keyboard navigation and update progress now remain clear while you read.',
    'Plain text remains text: <img src=x onerror=alert(1)> and <script>unsafe()</script>.',
  ].join('\n\n');
  const bodyLength = length - finalLine.length - 2;
  return `${seed.repeat(Math.ceil(bodyLength / seed.length)).slice(0, bodyLength)}\n\n${finalLine}`;
}

export function getMockUpdateFixtureOptions(search: string): MockUpdateFixtureOptions {
  const params = new URLSearchParams(search);
  if (params.get('mockUpdateState') === 'error-long') {
    return { updateCheckTransportError: makeMockReleaseNotes('long') };
  }

  const releaseNoteFixtures = ['none', 'short', 'medium', 'long', 'maximum'] as const;
  const fixture = releaseNoteFixtures.find(
    (candidate) => candidate === params.get('mockReleaseNotes'),
  );
  if (!fixture) return {};

  return {
    updateSnapshot: {
      state: 'available',
      current_version: '4.0.0-alpha.1-mock',
      available_version: '4.0.0-alpha.2-mock',
      channel: 'stable',
      release_notes: makeMockReleaseNotes(fixture),
    },
  };
}

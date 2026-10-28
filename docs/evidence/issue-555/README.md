# Wave C controlled UI trace

This trace compares the Wave B source at `2cee570758f2a28fe60fe920cf1a70095bbbcea0` with the Wave C source using the same deterministic fixture in `desktop/src/components/performance/WaveCProfile.test.tsx`.

## Input and measurement

- Vitest 5 with jsdom and the mock desktop bridge; 24-song normal and 500-song large library fixtures.
- Idle with diagnostics off and visible selection; playing with diagnostics off and visible selection distinct from Now Playing; playing with diagnostics on and selection outside the mounted virtualized page.
- Ten sequential `playback.snapshot` events per playing case, with the same session and song and `current_us` advancing by 100,000 µs each time. The idle case has no playback snapshot traffic.
- React `Profiler` commits for PlayerTransport, PlayerTrackInfo, DiagnosticsView and TrackTable; MutationObserver counts for child, attribute and text changes; `getElementById` calls; and mock bridge `searchSongs` / `setLibraryViewport` calls during the snapshot window.
- Setup calls are reported separately. Each case hydrated once (`searchSongs: 1`, `setLibraryViewport: 1`) before the measurement window.

The baseline was captured before the Wave C source edits at the commit above. The post-change capture was run on the final Wave C source with the exact command below. The fixture and measurement code were unchanged between captures.

## Before and after

Profiler values are React commits per scenario in the order `PlayerTransport / PlayerTrackInfo / DiagnosticsView / TrackTable`. Omitted component keys mean zero commits.

| Scenario | Commits before → after | DOM records before → after (`child / attributes / text`) | Active-descendant DOM queries before → after | Mock bridge calls during snapshots before → after |
| --- | --- | --- | --- | --- |
| Normal, idle, diagnostics off, visible selection | `0 / 0 / 0 / 0` → `0 / 0 / 0 / 0` | `0 / 0 / 0` → `0 / 0 / 0` | `0 → 0` | `0 → 0` |
| Normal, playing, diagnostics off, visible selection | `10 / 10 / 20 / 0` → `10 / 0 / 0 / 0` | `0 / 22 / 1` → `0 / 12 / 1` | `10 → 0` | `0 → 0` |
| Large, playing, diagnostics on, offscreen selection | `10 / 10 / 20 / 0` → `10 / 0 / 0 / 0` | `0 / 12 / 1` → `0 / 12 / 1` | `10 → 0` | `0 → 0` |

The ten PlayerTransport subtree commits while playing remain because the timeline must display each progress update. The action controls no longer subscribe to the full playback snapshot, while the timeline owns its progress selectors. PlayerTrackInfo and DiagnosticsView no longer commit for snapshot-only changes. TrackTable no longer queries the active-descendant element for each snapshot.

Semantic output stayed the same: the player title follows `Profile Song 1` while the selected row remains `Profile Song 0`; after ten snapshots progress is `1,000,000` µs; the five transport controls retain their labels and enabled states. In the large case, the selected song remains index 250 while Now Playing remains index 1, and the grid omits `aria-activedescendant` while that selected row is unmounted. The visible case keeps `aria-activedescendant` on the selected row.

## Reproduction and limits

Post-change command:

```powershell
bun run test -- --reporter=dot --silent=false src/state/store.test.ts src/components/player/PlayerTransport.test.tsx src/components/player/PlayerTrackInfo.test.tsx src/components/utility/DiagnosticsView.test.tsx src/components/tracks/TrackRow.test.tsx src/components/performance/WaveCProfile.test.tsx
```

Result: exit 0; 6 test files and 175 tests passed. The three `[wave-c-profile]` JSON records in stdout contain the raw post-change counts and semantic output.

Other focused checks, run from the repository root unless noted:

- From `desktop`: `bun run typecheck` — exit 0.
- `cargo test --manifest-path desktop/src-tauri/Cargo.toml --features tauri-test --lib start_prepare::` — exit 0; 14 passed, 284 filtered out, 0 skipped.
- From `desktop`: `bun run test:e2e -- --grep "row selection returns focus|virtualized grid navigation|virtualized library pages"` — exit 0; 3 Chromium tests passed, 0 skipped.

This is a browser-DOM profile with a mock bridge. Bridge counts are calls to the mocked frontend methods, not measured Tauri IPC latency. It does not measure native Windows rendering or playback latency. Native correctness is covered separately by the Rust playback integration tests, which use dry-run playback and send no physical input.

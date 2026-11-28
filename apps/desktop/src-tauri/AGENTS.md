# Native Desktop Test Guidance

- Use the pinned Rust wrapper from the repository root and retain the existing
  feature configurations for native desktop tests.
- A status snapshot is not acknowledgement of event publication. Assert
  per-session event order and status recovery while allowing successor
  admission to race terminal UI publication.
- For recovery and no-send assertions, acknowledge the exact foreground
  decision/cycle and count attempts/effects. Keep SendInput behavior mock-only
  in ordinary tests; native qualification follows the playback runbook.
- Scope shared Win32/global test hooks with reset guards and protect only the
  resources they actually share. Release and join owned workers on failure.
- Keep test-only observations out of production hot paths and preserve the
  security, authority, timing, and cleanup contracts.

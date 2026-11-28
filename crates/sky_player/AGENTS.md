# Playback Engine Test Guidance

- Run Rust commands through `node scripts/toolchain-pin.mjs exec --` from the
  repository root.
- Synchronize worker tests with an explicit checkpoint or acknowledgement.
  Use bounded deadlines as watchdogs, not as evidence that a decision ran.
- For negative assertions, acknowledge the processed decision/cycle before
  checking that no packet or replay occurred.
- Prove contested resources are held and release/join test workers on both
  success and failure. Keep real timer tests tied to the policy they exercise;
  use controlled timestamps for arithmetic-only deadline policy.
- Keep test observations behind test/test-support configuration and preserve
  production timing, target authority, cleanup, and the mock/physical-input
  evidence boundary.

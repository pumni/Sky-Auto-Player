# Desktop Browser Test Guidance

- Prefer accessible roles and names for interactive controls; use stable
  semantic slots only for layout geometry that has no accessible equivalent.
- Read related element geometry in one browser evaluation so assertions use a
  coherent layout snapshot. Keep the existing responsive bounds and use
  retryable final-state assertions only for genuinely eventual UI state.
- Preserve the interaction or race under test. A publication event must not be
  awaited before starting the successor whose race is being tested.
- Keep real debounce/response timing only when the test needs that timing to
  create or observe the contract, and state that reason near the wait.
- Keep test data and browser resources isolated. Preserve first-failure output
  and traces; do not change tolerances to make layout assertions pass.

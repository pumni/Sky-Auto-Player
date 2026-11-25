# Current UI captures

This directory is the **only recommended starting point** for agents who need a visual reference for the desktop app. Do not search the issue-specific screenshots as if they were current UI.

Check [current/manifest.json](current/manifest.json) before using an image. Only treat a capture as representing the checked-out desktop if `status` is `captured` and `desktop_tree_sha` matches `git rev-parse HEAD:apps/desktop` (from the repository root). The source commit may differ after a screenshot-only documentation commit; this tree identity avoids that false stale signal.

To replace them after a reviewed UI change, run from `apps/desktop/`:

```sh
bun run ui:refresh
```

This executes the existing opt-in Playwright visual capture, checks four canonical PNGs plus the
review-only Appearance settings state, and updates `current/` plus its manifest. It requires the UI
source and tests to be committed first. The images show deterministic browser mock state, **not a
native Windows/Tauri qualification**; the 640x448 overlay is browser-only. Native visual or input
claims require separate native evidence and should never be inferred from these images.

Historic visual reviews remain available through immutable commit-pinned links in their original evidence reports. Review screenshots, one-off comparison captures, traces, and logs belong in ignored `test-results/` or PR/release artifacts—not a continually expanding set of screenshot folders under `docs/evidence/`.

The WP1/WP2 work order, migration ledger, and performance captures are revision-bound history,
indexed in [HISTORY.md](../HISTORY.md). Current Base UI focus-guard coverage is described in the
[desktop type boundary](../desktop-type-boundary.md); the accepted unfiltered Axe reports remain at
[`axe-full-unfiltered-add-songs.json`](evidence/wp2/axe-full-unfiltered-add-songs.json) and
[`axe-focus-guard-adjudication.json`](evidence/wp2/axe-focus-guard-adjudication.json).

**D13 bundle policy:** emitted JavaScript size is advisory. There is no absolute or percentage
acceptance ceiling, and no chunk split or UI simplification is required to match an earlier build.
Preserve the approved visual behavior and judge performance with the revision-pinned WP2 profiles
linked from [HISTORY.md](../HISTORY.md); do not treat those browser measurements as native timing
qualification.

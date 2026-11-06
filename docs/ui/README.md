# Current UI captures

This directory is the **only recommended starting point** for agents who need a visual reference for the desktop app. Do not search the issue-specific screenshots as if they were current UI.

Check [current/manifest.json](current/manifest.json) before using an image. Only treat a capture as representing the checked-out desktop if `status` is `captured` and `desktop_tree_sha` matches `git rev-parse HEAD:apps/desktop` (from the repository root). The source commit may differ after a screenshot-only documentation commit; this tree identity avoids that false stale signal.

To replace them after a reviewed UI change, run from `apps/desktop/`:

```sh
bun run ui:refresh
```

This executes the existing opt-in Playwright visual capture, checks four PNGs, and replaces `current/` plus its manifest. It requires the UI source and tests to be committed first. The images show deterministic browser mock state, **not a native Windows/Tauri qualification**; the 640x448 overlay is browser-only. Native visual or input claims require separate native evidence and should never be inferred from these images.

Historic visual reviews remain available through immutable commit-pinned links in their original evidence reports. Review screenshots, one-off comparison captures, traces, and logs belong in ignored `test-results/` or PR/release artifacts—not a continually expanding set of screenshot folders under `docs/evidence/`.

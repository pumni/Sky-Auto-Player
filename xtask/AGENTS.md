# Xtask Verification Guidance

- Run xtask with the pinned Rust wrapper. Preserve exact child executable,
  argv, working directory, exit status, timeout, and output semantics.
- Prefer behavioral guards with independent positive and negative fixtures;
  accept only complete, explicitly supported tool banners and versions.
- Keep generation separate from verification, retain safety-critical checks,
  and do not replace a guard with a spelling-only or substring match.
- Preserve feature coverage, explicit skip/failure state, bounded owned
  subprocess cleanup, and the repository's security and release boundaries.
- Do not add a generalized runner or policy framework without a separate
  architecture decision.

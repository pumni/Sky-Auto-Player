# Agent Handoff — Context and Documentation Refactor

> Temporary work-order for this pull request only.
>
> This file exists so a local coding agent can receive the task from the PR branch without requiring a separate prompt bundle. **Delete this file before the PR is considered ready to merge.**

## Objective

Refactor Sky Auto Player's repository guidance and documentation surfaces so the project works cleanly with modern coding agents such as Codex CLI without overloading the model with duplicated instructions, stale implementation choreography, or vendor-specific context.

The target design is:

- **minimal instructions** — project instructions contain only durable, non-obvious constraints;
- **strong contracts** — security, architecture, release, timing, and other system rules live in canonical project documentation and executable checks;
- **scoped retrieval** — agents discover detailed context only when the task requires it;
- **executable truth** — source, direct tests, configuration, compiler checks, and CI are the primary current-state evidence;
- **Git as history** — completed implementation plans/work orders do not remain in the active documentation corpus unless they contain durable knowledge that has not yet been extracted.

This is a subtractive maintainability refactor, not an attempt to build a new agent framework.

## Working authority

Use the current repository state on this PR branch as the implementation base.

For this refactor:

1. Source, direct tests, executable configuration, and CI behavior are primary evidence.
2. `SECURITY.md` owns the security contract.
3. Current architecture/timing/distribution documents own durable system contracts.
4. Historical plans, work orders, old PR instructions, and handoffs are not authoritative for current behavior.
5. Do not change runtime/product behavior merely to simplify documentation.
6. Do not copy patterns from `openai/codex` mechanically. Adopt the underlying principles only when they fit this repository.

## Important design conclusions already agreed

### Keep a canonical `AGENTS.md`

Do **not** delete `AGENTS.md`.

Refactor it into a small, high-signal repository constraint layer. It should tell an agent only what cannot be safely inferred from normal source inspection.

Good candidates to keep:

- one-sentence repository/product orientation;
- concise repository/subsystem map;
- canonical verification commands;
- the non-obvious Windows gameplay-input/security boundary, with `SECURITY.md` explicitly authoritative;
- durable architectural dependency constraints that are easy to violate accidentally;
- the supported Rust/Bun toolchain constraint if it remains a real repository invariant;
- a concise external-mutation authority rule.

Remove or substantially reduce instruction choreography such as:

- “read the task first”;
- generic advice to inspect source/tests;
- detailed instructions for how the model should investigate or reason;
- statements explaining that the model should be autonomous;
- long lists of agent-framework concepts that must never be created;
- policy about how many context files the model should preload;
- meta language whose only purpose is to describe `AGENTS.md` as an instruction system.

The intended principle is:

> AGENTS.md tells the model what the repository cannot reliably tell it through source, tests, configuration, or canonical docs.

Do not introduce an arbitrary new byte/token budget unless there is concrete evidence that one is needed.

### Remove vendor-specific authority duplication

Delete `CLAUDE.md`.

Do not replace it with `CODEX.md`, `GEMINI.md`, `COPILOT.md`, or another vendor adapter.

The repository should have one canonical project-instruction surface rather than parallel vendor authorities.

### Retire the agent-context governance framework

The current Rust audit at:

`rust/xtask/src/audits/agent_context.rs`

is expected to be removed rather than rewritten into another prompt-governance system.

Also remove its registration/call sites from the xtask audit/check wiring.

Reasons:

- it enforces wording and file-shape policy rather than product correctness;
- it requires/limits vendor-specific context surfaces;
- it bans `.codex/` and nested `AGENTS.md`, even though those are legitimate scoped mechanisms in modern Codex workflows;
- it lexically bans plan/handoff naming patterns rather than validating software behavior;
- it creates second-order infrastructure whose purpose is to govern repository prompts.

Do **not** remove mechanical product/security checks merely because this audit is removed. Checks for Win32 APIs, input boundaries, zero-Python runtime/tooling requirements, release integrity, architecture invariants, etc. belong in executable verification and must remain intact.

A useful rule:

> If violating a rule can make the software incorrect or unsafe, enforce it with code/tests/CI. If violating a rule only makes repository guidance verbose or awkward, review it as documentation rather than building a prompt-governance framework.

### Do not ban scoped Codex mechanisms

Removing the audit should remove the blanket prohibition on:

- `.codex/`;
- nested `AGENTS.md`.

However, **do not add either mechanism as part of this refactor unless an actual current need is demonstrated**.

“Allowed” is not the same as “required.”

A future nested `AGENTS.md` should exist only for a genuinely local invariant that should not be loaded globally.

A future `.codex/skills` workflow should exist only when the same non-trivial procedure is repeated often enough to justify an on-demand skill.

### Preserve the website `llms.txt` surface

Do **not** remove `site/src/pages/llms.txt.ts` or its build/E2E contract merely as part of this repository-agent cleanup.

That endpoint is a public website/product-discovery concern, not automatically loaded project instruction context. Treat any decision about it separately.

### Add an external mutation boundary

Add a concise policy to `AGENTS.md` with this intent:

> Do not create, publish, close, merge, retarget, or otherwise mutate remote branches, pull requests, issues, releases, or other externally visible project state unless the user's current request explicitly authorizes that external action.

Keep this rule narrow. It must not force the agent to ask permission for ordinary local investigation, source edits, targeted tests, formatting, or other work already requested by the user.

For **this PR specifically**, the user has authorized the local agent to implement the refactor and commit/push those implementation changes to this PR's head branch. That authorization does **not** include merging, closing, retargeting, deleting the remote branch, publishing a release, or marking the PR ready for review unless the user separately asks.

## Documentation cleanup

The active tree currently contains multiple completed implementation handoffs and migration work orders. They create retrieval ambiguity because searches can surface old task choreography beside current contracts.

### Candidate completed implementation documents

Review each of these before deleting it:

- `docs/implementation-compact-playback-timing-ui.md`
- `docs/implementation-playback-start-failure-deadlock-fix.md`
- `docs/implementation-playerbar-error-recovery-ui.md`
- `docs/implementation-playerbar-lifecycle-spotify-transport.md`
- `docs/implementation-playerbar-ui-recovery.md`
- `docs/implementation-pr236-five-button-autoplay-dock-followup.md`
- `docs/implementation-real-game-input-reliability-late-down-tolerance.md`
- `docs/implementation-scheduler-reliability-diagnostics-ui.md`
- `docs/implementation-shuffle-autoplay-playback-modes.md`
- `docs/implementation-user-owned-symmetric-timing-margin.md`

### Candidate migration/work-order documents

Review each of these before deleting it:

- `docs/migration/ci-build-once-qualify-many-work-order.md`
- `docs/migration/ci-critical-path-latency-work-order.md`
- `docs/migration/ci-release-control-plane-work-order.md`
- `docs/migration/v4-codex-work-orders.md`

### Required classification procedure

Do **not** bulk-delete the files without reading them.

For each candidate:

1. Determine whether it is a completed/superseded task plan, handoff, or execution queue.
2. Identify any durable fact that is still true and not already represented in current source/tests/configuration/canonical documentation.
3. If such a fact exists, move or restate it in the correct durable owner:
   - architecture behavior -> current architecture document;
   - security rule -> `SECURITY.md` or executable security audit;
   - timing semantics -> timing contract/test;
   - release/update rule -> distribution/release contract;
   - irreversible design decision -> ADR when appropriate;
   - behavior -> direct test/source;
   - operational command that remains canonical -> current runbook/xtask help.
4. Avoid copying task chronology, implementation sequencing, reviewer instructions, PR numbers, “before editing read...” lists, or agent-specific choreography into durable docs.
5. Once all still-valid durable knowledge is accounted for, remove the completed task document from the active tree. Git history remains the archive.

The desired end state is not “fewer files at any cost”; it is “no stale task document competing with current truth.”

## `docs/INDEX.md`

Keep `docs/INDEX.md` as a compact documentation router.

Review it after retiring the completed documents:

- remove links to files that no longer belong in the active corpus;
- distinguish current contracts from historical evidence;
- avoid listing superseded migration material under a heading that implies current architecture;
- keep the router useful to humans as well as agents;
- do not turn it into another instruction manifest.

The router should help a reader answer: “Which current document owns this concern?”

It should not prescribe a model reasoning workflow.

## `SECURITY.md`

Keep `SECURITY.md` canonical.

It is reasonable to remove agent-specific wording such as statements about “agent guidance” if that wording no longer serves humans or the security contract, but do not weaken or rewrite the actual security requirements as part of documentation cleanup.

Preserve the executable security gate and current updater/release trust model.

## `CONTRIBUTING.md` and other top-level guidance

Audit references to the old context-governance model after the refactor.

A direct link to `AGENTS.md` is acceptable if useful, but do not describe a hierarchy of agent adapters or duplicate rules already owned elsewhere.

Prefer human-readable contribution guidance over instructions about model prompting.

## Historical provenance that should normally remain

Do not scrub historical evidence simply because it contains words such as “Codex”, “AGENTS.md”, or old PR references.

Examples that should generally remain unless they are actively misleading:

- `CHANGELOG.md` entries describing what happened at the time;
- release acceptance records;
- benchmark run IDs;
- performance evidence;
- commit/PR references used as provenance.

If a historical evidence file says an old mechanism is **currently** authoritative when it is no longer current, update that stale statement narrowly. Do not rewrite history for cosmetic consistency.

## Expected implementation style

Prefer a subtractive, reviewable diff.

Do not:

- introduce a new agent framework;
- introduce a generated context manifest;
- add a new policy engine to replace `agent_context.rs`;
- add a new dependency for this cleanup;
- change product/runtime behavior;
- rename unrelated files;
- perform broad formatting churn;
- rewrite historical evidence that is not misleading;
- remove `llms.txt` as part of this task;
- create nested `AGENTS.md` or `.codex/skills` speculatively.

If deleting `agent_context.rs` exposes a small, genuinely product-relevant check that has no other owner, move that check to the appropriate existing audit rather than preserving the entire context-policy abstraction.

## Verification

At minimum:

1. Run the narrow repository static verification after changing xtask wiring:

   `cargo xtask check static`

2. Run relevant Rust tests/checks for any modified xtask code.

3. Run the canonical full repository check when practical:

   `cargo xtask check all`

4. Search the resulting tree for stale **active** references to:
   - `CLAUDE.md`;
   - `agent_context`;
   - deleted implementation/work-order document paths;
   - claims that nested `AGENTS.md` or `.codex/` are forbidden.

5. Confirm the following remain intact:
   - security audit behavior;
   - `SendInput`-only gameplay-input enforcement;
   - zero-Python product/tooling enforcement if still canonical;
   - architecture boundary checks;
   - release/update integrity checks;
   - website `llms.txt` contract.

6. Review the final diff for unrelated changes.

If a full check cannot run in the local environment, report exactly what was run and why the remaining check could not run. Do not weaken checks merely to get a green local result.

## Definition of done

The refactor is complete when:

- `AGENTS.md` is a concise durable constraint/map layer rather than a model operating manual;
- `CLAUDE.md` is gone;
- `agent_context.rs` and its wiring are gone unless a clearly product-relevant fragment has been relocated to an appropriate existing audit;
- completed implementation/work-order docs have either been safely retired or retained only because they still own explicitly identified durable knowledge;
- any durable knowledge extracted from retired files has a clear canonical owner;
- `docs/INDEX.md` accurately routes to current documentation;
- no new vendor-specific instruction surface was introduced;
- `llms.txt` remains untouched by this concern;
- product/security/runtime behavior is unchanged;
- relevant checks pass;
- this temporary handoff file (`.github/agent-context-refactor-handoff.md`) has been deleted from the branch;
- the PR description is updated with the actual final scope, verification results, and any intentionally retained legacy document plus the reason it remains.

## Final agent report

Before handing control back to the user, report:

- files removed;
- files materially rewritten;
- durable knowledge moved and its new owner;
- any candidate historical document intentionally retained and why;
- checks run and exact results;
- remaining risks or ambiguities;
- confirmation that this temporary handoff file was removed;
- confirmation that the PR was **not merged/closed/retargeted/marked ready** unless the user explicitly authorized that action.

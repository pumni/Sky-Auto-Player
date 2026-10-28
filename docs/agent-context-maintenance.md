# Agent Context Maintenance

This is an **on-demand maintenance procedure**, not a startup instruction
bundle. The working agent guide is [../AGENTS.md](../AGENTS.md); the documentation
router is [INDEX.md](INDEX.md). Current model names, effort levels, and tool
availability belong to the agent runtime, not a version-pinned repository prompt.

## Goal and scope

Maintain correct, task-relevant project context for Codex CLI, Claude Code,
and other capable coding agents without duplicating architecture in multiple
instruction files. Audit instructions when a model generation changes, when
repository ownership changes, or when specific agent failures suggest stale
guidance. A shorter prompt is not a success metric by itself.

Default audit scope covers version-controlled agent instruction surfaces,
including root/scoped `AGENTS.md`, any committed `CLAUDE.md`, rules,
skills, commands, subagent prompts, tool descriptions, prompt-bearing templates,
and the current documentation linked by those instructions. Inspect historical
documents for classification and reference integrity; do not treat their
contents as executable agent instructions. User-home instructions, local
settings, secrets, and private MCP credentials are outside this scope unless
the operator separately authorizes review of a specific safe file.

This repository intentionally uses one shared `AGENTS.md` root instruction
source, with [AGENTS.md](AGENTS.md) scoped to documentation edits. Do not
introduce a second copy in `CLAUDE.md` just for tool branding. Claude Code
versions that support `AGENTS.md` can use the shared instructions; verify
the effective loaded instructions if the local installation has overrides,
imports, plugins, or a different project-instructions configuration.

## Source authority and document lifecycle

| Category | Typical sources | Handling |
| --- | --- | --- |
| Executable/current implementation | Rust/TypeScript source, tests, Tauri configs, pinned toolchain, CI workflows | Inspect exact SHA; do not infer runtime behavior from old reports |
| Intended current contract | [architecture.md](architecture.md), [rt-dispatch-architecture.md](rt-dispatch-architecture.md), [timing-principles.md](timing-principles.md), [distribution-and-update.md](distribution-and-update.md), [../SECURITY.md](../SECURITY.md) | Keep project-specific invariants and enforcement evidence |
| Current operator/runbook | [v4-release-authority.md](v4-release-authority.md), [v4-release-execution-topology.md](v4-release-execution-topology.md), [v4-updater-key-custody.md](v4-updater-key-custody.md) | Retain exact safety/ordering rules for fragile operations |
| Task research and methods | [desktop-startup-benchmark.md](desktop-startup-benchmark.md), [prepare-whole-stream-timing-advisory.md](prepare-whole-stream-timing-advisory.md) | Read only when task needs the technique; distinguish modeled from measured |
| Historical evidence | `docs/adr/`, `docs/evidence/`, `docs/releases/`, and historical records listed in [INDEX.md](INDEX.md) | Preserve SHA, dates, inputs, caveats, and observed results; do not promote to current authority |

When code and a current contract disagree, identify the contradiction and
the boundary owner. Do not silently "fix" a security, timing, or release
requirement by deleting it from documentation.

## Audit procedure

1. **Establish identity.** Record repository, base SHA, target agent/model
   family, version-controlled prompt surfaces found, and out-of-scope local
   configuration. No model-specific claims should be guessed from the repo.
2. **Inventory effective instructions.** Inspect every tracked `AGENTS.md`,
   `CLAUDE.md`, rules, skills, command, and agent definition. Follow *explicit*
   imports only when safe. Search the source tree for prompt assembly/tool
   descriptions before asserting that none exist. Do not read credential
   files or private settings wholesale.
3. **Trace provenance.** For suspicious instructions, use relevant
   `git log`, `git blame`, code/tests, and existing issues/evidence to
   establish which behavior each rule protects. An old-looking idiom alone
   is not proof of harm.
4. **Audit authority and drift.** Verify paths, named commands, configuration,
   tool ownership, and the current/historical classification from
   [INDEX.md](INDEX.md). Separate stale facts, true conflicts, and
   merely repeated but consistent explanations.
5. **Find behavioral fossils.** Flag obsolete reasoning rituals, generic
   exhortations, repeated pressure language, forced workflows for ordinary
   coding tasks, retired-model workarounds, examples that freeze prior model
   behavior, and duplicate agent roles. Keep hard project constraints,
   tool/API contracts, exact release procedures, and demonstrated defenses.
6. **Prepare changes.** For each finding record `file:line`, evidence,
   confidence (high/medium/low), action, and risk. Edit high-confidence
   contradictions and stale references; validate medium-confidence edits
   behaviorally; leave low-confidence style opinions as observations.
   Keep a reviewable diff isolated from product code.
7. **Verify.** Check relative Markdown links and instruction-file references,
   diff for accidental security/contract loss, inspect the Git diff,
   and run the appropriate repository gates. Execute small task probes
   from [agent-context-evals.md](agent-context-evals.md) on both old and new
   instructions when agent behavior is materially changed. Record what was
   actually run, platform, SHA, and unresolved coverage.

A documentation-only audit does not qualify realtime dispatch or installer
performance. Full Windows acceptance must only be claimed for the exact
code, binary, environment, and runs that produced it.

## Editing policy

- Keep the always-loaded root instructions short and specific to permanent
  boundaries, workflow permissions, verification, and the documentation router.
- Put module semantics in their owning contracts or code; put reusable audit
  procedures in this file, opened only for context-maintenance work.
- Use a scoped `AGENTS.md` only for genuinely different directory-specific
  behavior, not a copy of root policies. If an instruction needs evaluation
  rather than universal enforcement, keep it here instead of loading it on
  every coding task.
- Do not manufacture a fixed token limit, mandatory planning phases, extra
  subagent roles, or a requirement to read every document before every edit.
- Do not clean up historical records solely to make them shorter. Prefer
  classifying them in [INDEX.md](INDEX.md) so agents reach current contracts first.
- Do not add model-specific instructions without a reproduced behavioral
  problem and a clear removal/retest condition. The model's own tool choice
  and reasoning effort are configured by the calling runtime, not prose.
- When tooling can check an invariant deterministically, prefer the
  established Rust/Bun validation pipeline; avoid new Python tooling.

## Findings and evidence format

For each audit record:

```text
Base SHA / target tool and model / scope / excluded configuration
Finding ID, severity, confidence, file:line, existing rule or claim
Repository evidence + why it harms or could confuse the target agent
Action: keep | rewrite | move | remove | flag
Patch path, validation command/result, behavioral probe (if relevant)
Residual risk / proposed follow-up owner
```

A clean surface should remain unchanged. A note that a model may benefit
from fewer instructions is not sufficient evidence to delete a project-specific
safety rule.

## Method references

- [Anthropic prompt-audit method](https://github.com/anthropics/skills/blob/main/skills/claude-api/shared/prompt-audit.md)
  (an audit methodology; not a copy of Claude Code's proprietary internals).
- [OpenAI guidance on modernizing coding-agent instructions](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra).
- [Codex agent workflows](https://developers.openai.com/cookbook/examples/codex/iterating-development-workflows-with-codex).

Review these sources again when a later model generation changes their advice;
the repository's safety and runtime contracts remain project-owned.

# Maintaining Agent Context

Use this reference when auditing agent guidance, not during ordinary coding.
The goal is better task outcomes with **less unnecessary context and fewer
constraints on the model's judgment**, not a target token or file count.

## What belongs in context

Keep a small always-loaded [AGENTS.md](../AGENTS.md): project-specific
invariants, permissions, surprising conventions, and useful validation
commands. Code and direct tests explain ordinary implementation details;
[INDEX.md](INDEX.md) points to contracts relevant to a task. Historical
plans and benchmarks should be retrievable, not automatically loaded.

Current Claude Code can read `AGENTS.md` directly when configured to do so.
Do not add a duplicate `CLAUDE.md` for branding. If the local Claude
installation loads different project instructions, check `/context` and
its Project instructions setting; an ancestor `CLAUDE.md` or
`CLAUDE.local.md` can take precedence by default.

## Questions to ask in an audit

- **Does this text reach the model?** Look at tracked instruction files,
  explicit imports, skills/rules/commands, subagent prompts, tool
  descriptions, and actual prompt-building code. Do not read secrets,
  personal settings, or credential-bearing files for a repository audit.
- **Is it accurate now?** Check referenced paths, commands, module ownership,
  and the applicable source/CI SHA. Distinguish active contracts from
  revision-scoped evidence; trace suspicious rules to tests or history.
- **Is it necessary?** Keep rules the model cannot infer from code. Remove
  generic exhortations, repeated reminders, task-independent document
  preloads, scripted reasoning or planning, and duplicates that change no
  behavioral boundary.
- **Is it in the right place?** Universal instructions belong in the root
  guide, directory-specific constraints in scoped guidance, task procedures
  in on-demand references. Precise API/tool contracts belong in code or tool
  definitions. Use existing Rust/Bun checks for deterministic invariants
  rather than hoping prose will enforce them.
- **What changes if it disappears?** Preserve verified defenses for input
  security, authored timing, updater trust, and release operations. If
  deletion could alter an important behavior, compare actual task outcomes
  before removing it. Stylistic guesses about new models are not proof.

More capable models can select tools, explore code, plan, and test without a
written itinerary. Explicitly define outcomes and exceptional boundaries,
then let them choose the route. Neither extra subagents nor new skills are
automatically an improvement.

## Reviewing a proposed change

Provide the affected files, source evidence, intended effect, and residual
uncertainty in a reviewable diff. Test changed Markdown references and
existing repository checks where relevant. When agent behavior is in
question, compare a small representative sample of tasks from
[agent-context-evals.md](agent-context-evals.md) with the old and new
guidance. Record observable results, tool use, effort/model settings,
and checks actually run; do not declare an evaluation passed from a
self-reported answer.

Do not rewrite historical measurements or remove safety obligations solely
to shorten documents. A clean prompt surface needs no ritual cleanup.

## Background

- [OpenAI: Rethinking skills and prompts](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra)
- [Anthropic: Effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [Anthropic: Prompt audit methodology](https://github.com/anthropics/skills/blob/main/skills/claude-api/shared/prompt-audit.md)
- [Claude Code project instruction loading](https://code.claude.com/docs/en/memory)

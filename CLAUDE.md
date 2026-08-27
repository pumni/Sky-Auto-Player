# Claude Code

Read `@AGENTS.md` before non-trivial work; it is the vendor-neutral repository contract.

Use `docs/INDEX.md` as the context router. Search relevant source and direct tests before opening
deeper documentation. Do not preload `docs/plan/`, `docs/archive/`, performance baselines,
historical releases, or old implementation notes.

For broad investigation, use an isolated subagent when it keeps raw exploration out of the main
context. Return only verified paths, facts, risks, and recommended next actions.

When compacting, preserve the objective and acceptance criteria, relevant or modified paths,
verified commands/results, established decisions, unresolved blockers, and the next action. Drop raw
exploration and obsolete hypotheses.

Do not introduce Claude-specific rules, hooks, skills, prompt packs, or context machinery unless
repeated task failures and an evaluation justify them.
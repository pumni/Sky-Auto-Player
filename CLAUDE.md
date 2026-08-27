# Claude Code

Use `@AGENTS.md` as the shared repository contract. Do not create Claude-specific authority on top of
it.

Start from the task and relevant source/tests. Use `docs/INDEX.md` to retrieve current documentation
only when it is needed. Historical rationale lives in Git history or explicitly archived evidence and
should not enter the working context unless the task calls for it.

For broad exploration, an isolated subagent is useful when it keeps raw search output out of the main
context. Return only verified paths, facts, risks, decisions needed, and recommended next actions.

When compacting, preserve the objective and acceptance criteria, relevant or modified paths, verified
commands/results, established decisions, unresolved blockers, and the next action. Drop raw
exploration and obsolete hypotheses.

Do not add Claude-specific hooks, skills, prompt packs, memory files, or context machinery unless
repeated task failures and an evaluation justify them.

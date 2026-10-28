# Agent Context Audit — 2026-10-28

**Repository:** `pumni/Sky-Auto-Player`  
**Baseline `main`:** `24fb9afa817e14df667f29b1bd1fee9832069dec`  
**Change branch:** `codex/context-engineering-audit-20261028`  
**Target:** modern Codex CLI and Claude Code coding agents, model-neutral shared instructions.  
**Applied change commits:** `3e0554130124b94646061c45d3b64e2bd9fff731` and the subsequent audit-record commit.  
**Scope:** version-controlled agent instructions and the documentation router,
with a repository-wide Markdown documentation inventory and reference check.
**Excluded:** local/user-level agent settings, secrets, private MCP settings,
binary artifacts, untracked state, actual model execution, and Windows native
runtime qualification.

## Executive finding

The existing root `AGENTS.md` was already short and project-specific. No
unjustified "think step by step", forced reasoning trace, always-on multi-agent
orchestration, or retired-model prompt scaffolding was found in the tracked
instruction surface. The main issue was not length: the documentation router
presented revision-specific migration/ownership evidence together with
current contracts, making it easy for agents to treat the former as binding
current behavior. The chosen fix is authority classification and
progressive disclosure rather than indiscriminate deletion.

No application source, Rust/Tauri logic, CI pipeline, signed-release policy,
saved historical evidence, or release artifact is changed.

## Inventory and provenance

The baseline Git tree contains **105** tracked Markdown documents below
`docs/`:
**36** at its root, **17** ADRs, **14** under `evidence/`, and
**38** under `releases/`. The tracked agent-instruction inventory had only
the root `AGENTS.md`, without tracked `CLAUDE.md`, `.claude/` commands,
rules, agents, or skills. There is no justification to create duplicate
vendor-specific copies of the same instruction file.

The agent-guide history includes `0a3b0cf70524` (2026-10-23, context/evidence
reconciliation) and `d2ea76b559c3` (2026-09-14, context modernization).
The documentation router was recently changed for UI evidence and release
proofs, including `0a997eb714e5` (2026-10-27) and
`0210ceba383f` (2026-10-26). These commit listings establish timing
and provenance, **not** that any line was originally a workaround for an
obsolete model. No unsupported model-era claim was used to delete a rule.
A line-by-line `git blame` was not run in this audit environment.

## Findings (baseline file lines)

| ID | Severity / confidence | Location at baseline | Evidence | Applied disposition |
| --- | --- | --- | --- | --- |
| C-01 | Medium / high | `docs/INDEX.md:8-10` | `native-playback-ownership.md` was grouped alongside current architecture references, but its text describes a before/after extraction and historical parity evidence. | Move it to a clearly labeled historical group; point current ownership questions at `architecture.md`. |
| C-02 | Medium / high | `docs/INDEX.md:23-26` | `v4-hosted-signing-proof.md` appeared inside the current release/security reading list; its own opening identifies a historical Wave A snapshot and migration proof. | Reclassify as revision-bound evidence; direct current operational decisions to release authority/execution topology runbooks. |
| C-03 | Medium / medium | `docs/INDEX.md:1-4`, `docs/README.md:1-4` | The router and description did not make document lifecycle legible at the point of selection, increasing the chance of wrong-source retrieval. | Add a task-driven table, explicit authority resolution, and historic-reference section without removing existing canonical documents. |
| C-04 | Low / medium | `AGENTS.md:21-37`, `CONTRIBUTING.md:8-20` | The root guide referenced the docs router but did not describe the selective-read and documentation-classification boundary as explicitly as the repository's current documentation warranted. | Reword for task-scoped reads and link an on-demand audit method. Preserve security, ownership, Rust/Bun, and test contracts. |
| C-05 | Medium / medium | `docs/` (no scoped agent guide at baseline) | Documentation edits involving historical release/timing evidence have different preservation requirements from ordinary coding. | Add a short `docs/AGENTS.md` scoped to documentation edits, not a global requirement to load all docs. |
| C-06 | Low / medium | No tracked context-audit method or behavioral probes | No durable way to distinguish a later model-specific prompt preference from verified repository authority; general model upgrades may lead to ad hoc prompt churn. | Add optional `agent-context-maintenance.md` and `agent-context-evals.md`; neither is an automatic startup instruction or a CI gate. |

## Kept deliberately

- Safety exclusions and Windows `SendInput`-only boundary in root instructions
  and `SECURITY.md`. These are product-specific invariants, not an obsolete
  model refusal workaround.
- Rust/Bun toolchain, architectural ownership, pinned release integrity,
  signer custody, and `cargo xtask` verification. These are constraints
  backed by the repository and/or safety-sensitive workflows.
- All previous ADRs, release notes, CSV/JSON measurement evidence, immutable
  SHA identities, historical names, and qualification caveats. Revision-bound
  evidence must not be rewritten as if measured on current HEAD.
- Exact release and timing procedures, even when long, because correctness and
  destructive-operation safety outrank token-minimization heuristics.
- Working duplication where contracts agree: not every repeated statement is
  prompt cruft. No mass compression or automatic historical move/rename is
  justified without source-specific failures.
- No `CLAUDE.md` mirror, skill zoo, compulsory planning harness, or
  model-version-specific instructions. These would expand always-loaded
  context without an observed gap.

## Documentation classification

The **current contract** set remains owned by
`architecture.md`, `rt-dispatch-architecture.md`,
`timing-principles.md`, `hold-frame-model.md`,
`distribution-and-update.md`, `v4-release-authority.md`,
`v4-release-execution-topology.md`, `v4-updater-key-custody.md`,
`v4-authenticode-provider-seam.md`, `ci-validation-topology.md`,
`library-v1-contract.md`, `desktop-workbench.md`, and the other
task-specific sources named by [../../INDEX.md](../../INDEX.md).
This classification is a routing policy, not a claim that every sentence of
each contract was runtime-validated in this audit.

Root-level evidence documents, including `r3-*`, `gate-*`,
`prepared-*-evidence`, `phase-7-301-evidence.md`,
`maintenance-test-pipeline-report.md`,
`dispatch-forensics-cost-qualification.md`,
`native-playback-ownership.md`, and `v4-hosted-signing-proof.md`,
remain at stable paths and are clearly labeled as bounded evidence.
The 17 ADRs, 14 nested evidence documents, and 38 release records are also
historical/reference material. A task may intentionally read them to trace
a decision; they do not silently supersede current contracts.

## Validation evidence and gaps

- The version-controlled tree and Markdown documentation inventory were
  enumerated at baseline `24fb9afa817e14df667f29b1bd1fee9832069dec`.
- Relative Markdown references were inspected across the baseline documentation
  inventory. A path-level scan reported no missing local targets for standard
  inline Markdown links; this does **not** validate URL anchors, links in
  code blocks, remote links, or the meaning of linked documents.
- Newly written/modified files were checked for Markdown relative-link
  target existence and trailing whitespace before the first commit.
- The first GitHub commit was compared against baseline: seven changed files,
  all documentation/agent context. Its branch was ahead by one commit and not
  behind at comparison time.
- The second commit adds this review record and retains the pre-existing
  Graphite final polish evidence link in the historical section of the router.
- **Not run:** `cargo xtask check all`, executable tests, Claude
  `/doctor prompt-audit`, Codex/Claude comparative task probes, native
  Windows acceptance, performance baselines, and full semantic proof of
  every assertion in all 105 docs. Their status must not be described as passed.
- User-level Claude/Codex instructions and hidden plugin/skill configuration
  were not accessed. Effective local session context may differ from
  repository-tracked instructions.

## Merge review

Review the exact branch diff. Confirm that
`AGENTS.md` keeps the original security, input, release, naming, and
verification boundaries; `docs/AGENTS.md` only applies to docs; and the
new router preserves links to prior evidence. Run
`cargo xtask check all` in the supported environment before merge and
run targeted [context probes](../../agent-context-evals.md) when practical
to validate agent behavior rather than relying on prompt self-report.

## Methodology sources

- [Anthropic prompt-audit methodology](https://github.com/anthropics/skills/blob/main/skills/claude-api/shared/prompt-audit.md).
  This public skill is not the proprietary Claude Code command implementation.
- [OpenAI on model-era agent instruction cleanup](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra).
- [Claude Code AGENTS.md instructions](https://code.claude.com/docs/en/claude-md).

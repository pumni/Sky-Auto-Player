## Outcome

What changed, and what user/repository outcome does it produce?

## Scope

What areas were intentionally touched? Call out any intentional behavior, architecture, security, or
release-contract change.

## Evidence

List the checks or observations that directly support the change.

- `command` → result

## Risk

Describe meaningful behavioral, security, timing, packaging, or release risks and how they are
bounded. Write `None identified` when appropriate rather than inventing ceremony.

## Specialized verification

Record any Windows, packaged-build, timing/latency, benchmark, updater, or manual evidence required
by the changed boundary. If none applies, say so.

## Checklist

- [ ] Intentional behavior/contract changes are documented where current active docs would otherwise become inaccurate.
- [ ] Relevant repository checks pass, or exceptions are explained above.
- [ ] Security-sensitive changes preserve the canonical `SECURITY.md` boundary.
- [ ] The final diff contains no unrelated churn.

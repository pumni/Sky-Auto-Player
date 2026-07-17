## Summary

<!-- One paragraph: what this PR does and why. -->

## Validation (altitude table)

Apply the narrowest gate that matches your change scope per `AGENTS.md` §Validation. Mark each entry you'll run:

- [ ] `uv run ruff check .`
- [ ] `uv run pyright`
- [ ] `uv run pytest -m "<markers>"`
- [ ] `uv run --env-file .env python scripts/audit_security_mandates.py`
- [ ] `uv run --env-file .env python scripts/audit_free_threaded_wheels.py`
- [ ] `uv run --env-file .env python -m build_app` (only if `Sky-Player.spec` / `src/build_app.py` changed)

## AGENTS.md priority stack

- [ ] I read `AGENTS.md` Priority Stack (P0 security → P1 enforced config → P2 local evidence → P3 task intent). My change does not violate any rule above `P3`.
- [ ] If my change touched a `PORTING_GUIDE.md §6` boundary, I confirmed `AGENTS.md` still wins.
- [ ] I did not introduce new dependencies unless I justified them in the PR description.

## Change scope

<!-- Surgical, focused, no broad rewrites. List the files touched and the rationale per file. -->

## Tests

<!-- New tests added? Existing tests updated? Targeted refactors covered? -->

## Risk and rollback

<!-- How risky is this? How do we revert if it breaks? -->

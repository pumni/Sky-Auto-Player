name: Bug report
about: Something is broken; the dispatch engine, the picker, the build, anything.
title: "[Bug] "
labels: ["bug"]

---

**Describe the bug**

A clear and concise description.

**To reproduce**

```powershell
# Steps, commands, song file (if relevant), config snippet
```

**Expected behaviour**

What you expected to happen.

**Screenshots / logs**

If applicable, paste from `logs/` or `git diff` snippets.

**Environment**

- Sky Player version (`--version` or `git describe --tags`):
- Windows build (`winver`):
- Terminal (Windows Terminal / cmd / VS Code):
- Python interpreter (`.python-version`):
- `uv run --env-file .env python scripts/audit_free_threaded_wheels.py` output:

**Security relevance**

- [ ] No — this is purely functional.
- [ ] Touches `AGENTS.md` P0 mandates (memory, hooks, anti-cheat, input validation).
- [ ] Crashes or hangs the dispatch loop.

**Validation already run**

- [ ] `uv run ruff check .`
- [ ] `uv run pyright`
- [ ] `uv run pytest`
- [ ] `scripts/audit_security_mandates.py`

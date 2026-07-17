name: AGENTS.md P0 report
about: A confirmed or suspected violation of the security mandates in AGENTS.md (memory tampering, hooks, DLL injection, etc.).
title: "[P0] "
labels: ["security", "p0"]

---

**Do not include exploit-ready code in a public issue.** For vulnerability-grade findings, email security@pumni.dev instead.

**Summary**

What looks off. File path + line, suspected API, suspected caller's intent.

**Where**

- File(s):
- Line(s):
- Rule from `scripts/audit_security_mandates.py`:

**Why it matters**

Briefly: which P0 mandate is at risk? (`NO GAME TAMPERING`, `SENDINPUT ONLY`, `STRICT VALIDATION`).

**Reproducer**

The audit output itself, or a minimal command:

```powershell
uv run --env-file .env python scripts/audit_security_mandates.py
```

**Suggested fix**

If obvious, link to a candidate approach (e.g. "drop the `_hook` field, switch `is_virtual_key_down` polling"). If subtle, just describe.

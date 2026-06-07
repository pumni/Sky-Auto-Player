## Project Rules

This is a Windows 11 25H2 Sky music playback helper.

# Hard constraints:

- Do not modify game files.
- Do not read game memory.
- Do not bypass anti-cheat or security systems.
- Use Windows SendInput only.
- Preserve current CLI behavior unless explicitly changed.
- Prioritize timing correctness, testability, and strict validation.
- Avoid broad rewrites without tests.

# Coding rules:

- Python 3.14.3
- Type hints required.
- Prefer dataclass(frozen=True, slots=True) for domain models.
- Avoid globals in new code.
- Scheduler must be pure and unit-testable.
- Windows backend must be isolated behind an interface.

# Workflow rules:

- Use `uv run <command>` for all Python executions (run, test, lint, typecheck).
- Do NOT use `pip install` inside .venv; use `uv add <package>` or `uv add --dev <package>`.
- Use `uv sync` to install/update project dependencies.
- Do NOT manually activate .venv in scripts or CI; `uv run` handles it.

# Terminal rules

- This project runs on Windows 11 using PowerShell 7.
- Prefer PowerShell-compatible commands.
- Do not use Bash-only syntax such as `export VAR=value`, `rm -rf`, or `cp -r`.
- For deleting folders, use `Remove-Item -Recurse -Force`.

# Preferred local tools

The following CLI tools are available on this Windows machine:

- `rg` for fast text/code search.
- `fd` for fast file discovery.
- `bat --paging=never` for reading files with line numbers/syntax highlighting.
- `jq` for inspecting and transforming JSON.

Prefer these tools before slower built-in alternatives:

- Use `rg` before `Select-String`, `findstr`, or manual recursive scans.
- Use `fd` before `Get-ChildItem -Recurse`.
- Use `bat --paging=never` before `type`, `cat`, or `Get-Content` when reading source/config files.
- Use `jq` for JSON instead of manual string parsing.

This project runs on Windows 11 with PowerShell 7. Prefer PowerShell-compatible commands.

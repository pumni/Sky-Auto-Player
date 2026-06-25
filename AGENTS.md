# Project Rules

This is a Windows 11 Sky music playback helper.

## Hard constraints

- Do not modify game files.
- Do not read game memory.
- Do not bypass anti-cheat or security systems.
- Use Windows `SendInput` only.
- Preserve current CLI behavior unless explicitly changed.
- Prioritize timing correctness, testability, and strict validation.
- Avoid broad rewrites without tests.

## Coding rules

- Use Python 3.14.3.
- Type hints are required.
- Prefer `@dataclass(frozen=True, slots=True)` for domain models.
- Avoid globals in new code.
- Keep the scheduler pure and unit-testable.
- Isolate the Windows backend behind an interface.
- Prefer small, focused changes over large rewrites.
- Do not introduce new dependencies unless they are clearly justified.

## Workflow rules

Use `uv run <command>` for all Python executions, including run, test, lint, and typecheck.

Preferred commands:

```powershell
uv run pytest
uv run ruff check .
uv run pyright
uv run python -m app
```

Dependency rules:

- Use `uv sync` to install or update project dependencies.
- Use `uv add <package>` for runtime dependencies.
- Use `uv add --dev <package>` for development dependencies.
- Do not use `pip install` inside `.venv`.
- Do not manually activate `.venv` in scripts, local commands, or CI. `uv run` handles the environment.

## Command usage rules

**Shell: PowerShell 7 (`pwsh`).** All commands run in PS7. `&&` and `||` chaining work. Use `;` for sequential steps only when exit-code propagation is not needed.

### General principles

- Prefer narrow, targeted commands over broad recursive output.
- Search before reading files.
- Avoid dumping large files into the terminal.
- Skip generated folders: `.venv`, `dist`, `build`, `__pycache__`, `.pytest_cache`, `.ruff_cache`, `.uv-cache`.
- Prefer commands that return file paths, line numbers, and small surrounding context.
- When reading files, prefer `--line-range` over reading the whole file.
- Keep terminal output small to preserve context and reduce token usage.

### Fast search and discovery

Use `rg` for text/code search; use `fd` for file discovery. Prefer both over PS7 built-ins.

```powershell
# Search text
rg -n "<pattern>" src
rg -n -C 3 "<pattern>" src
rg --files -g "*.py" src

# Find files
fd -e py
fd test
```

Do NOT default to `Select-String`, `findstr`, or `Get-ChildItem -Recurse`.

Exclude project noise:

```powershell
rg -n "<pattern>" -g "!.venv" -g "!dist" -g "!build" -g "!__pycache__" -g "!.pytest_cache" -g "!.ruff_cache"
fd "<pattern>" -E .git -E .venv -E dist -E build -E __pycache__ -E .pytest_cache -E .ruff_cache
```

### Reading files

Use `bat --paging=never` with a line range whenever possible.

```powershell
bat --paging=never --line-range 1:60 pyproject.toml
bat --paging=never --line-range 40:120 src/scheduler.py
```

Recommended flow — search first, then read only the relevant range:

```powershell
rg -n "TargetSymbol" src
bat --paging=never --line-range 80:160 src/example.py
```

### JSON

Use `jq` for JSON inspection. Extract only needed fields.

```powershell
jq -r ".tool.uv" pyproject.toml
jq -r ".tool.ruff" pyproject.toml
jq "." config.json
```

Avoid printing entire large JSON files.

### Available CLI tools

| Tool | Use for |
| ---- | ------- |
| `rg` | Fast text and code search |
| `fd` | Fast file discovery |
| `bat --paging=never` | Reading source/config files with line numbers |
| `jq` | Inspecting and transforming JSON |

Prefer these over slower PS7 built-ins (`Select-String`, `Get-ChildItem -Recurse`, etc.).

## Testing and validation

Before completing a change, run the smallest relevant validation first.

Examples:

```powershell
uv run pytest tests/path/to/test_file.py
uv run pytest -k "<keyword>"
uv run ruff check .
uv run pyright
```

For behavior-sensitive changes, prefer adding or updating tests before changing implementation.

For scheduler changes:

- Keep logic pure.
- Add unit tests for timing edge cases.
- Avoid depending on wall-clock time directly inside core scheduling logic.

For Windows backend changes:

- Keep platform-specific code isolated.
- Validate inputs strictly before calling Windows APIs.
- Do not mix scheduling logic with `SendInput` implementation details.

## Change discipline

- Preserve current CLI behavior unless explicitly changed.
- Do not perform broad rewrites without tests.
- Do not change unrelated files.
- Keep diffs focused and easy to review.
- Prefer explicit validation and clear error messages over implicit fallback behavior.
- If a command fails, inspect the error and fix the root cause instead of retrying blindly.

## Security and safety

- Do not modify game files.
- Do not read or inspect game memory.
- Do not bypass anti-cheat, security, or integrity systems.
- Do not add hooks, injection, memory scanning, or process tampering.
- Use only Windows `SendInput` for input simulation.
- Keep user input validation strict.
- Avoid logging sensitive local paths or unnecessary environment details.

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

### General principles

- Prefer narrow, targeted commands over broad recursive output.
- Search before reading files.
- Avoid dumping large files into the terminal.
- Avoid reading generated files, dependency folders, caches, build outputs, or virtual environments unless explicitly needed.
- Prefer commands that return file paths, line numbers, and small surrounding context.
- When reading files, read only the relevant line range whenever possible.
- Keep terminal output small enough to preserve context and reduce token usage.

### Fast search and discovery

Use `rg` for text and code search.

Prefer:

```powershell
rg -n "<pattern>"
rg -n -C 3 "<pattern>"
rg --files
rg --files -g "*.py"
```

Use `rg` before slower alternatives such as:

```powershell
Select-String
findstr
Get-ChildItem -Recurse
```

Use `fd` for file or directory discovery when pattern-based discovery is needed.

Prefer:

```powershell
fd "<pattern>"
fd -e py
fd test
```

Exclude heavy folders when useful:

```powershell
rg -n "<pattern>" -g "!node_modules" -g "!.git" -g "!.venv" -g "!dist" -g "!build" -g "!.expo" -g "!__pycache__" -g "!.pytest_cache" -g "!.mypy_cache" -g "!.ruff_cache"

fd "<pattern>" -E .git -E .venv -E node_modules -E dist -E build -E .expo -E __pycache__ -E .pytest_cache -E .mypy_cache -E .ruff_cache
```

### Reading files

Use `bat --paging=never` for source and config files.

Prefer line ranges:

```powershell
bat --paging=never --line-range 1:160 pyproject.toml
bat --paging=never --line-range 120:220 src/module.py
```

Avoid reading entire large files unless necessary.

Recommended flow:

```powershell
rg -n "TargetSymbol" src
bat --paging=never --line-range 80:160 src/example.py
```

### JSON

Use `jq` to inspect and transform JSON.

Prefer extracting only needed fields:

```powershell
jq -r ".scripts" package.json
jq -r ".dependencies, .devDependencies" package.json
jq -r ".tool" pyproject.json
```

Avoid printing entire large JSON files unless necessary.

### Preferred local tools

The following CLI tools are available on this Windows machine:

- `rg` for fast text and code search.
- `fd` for fast file discovery.
- `bat --paging=never` for reading files with line numbers and syntax highlighting.
- `jq` for inspecting and transforming JSON.

Prefer these tools before slower built-in alternatives.

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

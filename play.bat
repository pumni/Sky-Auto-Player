@echo off
set "PACKAGED_CORE=%~dp0Sky-Auto-Player-Core.exe"
if exist "%PACKAGED_CORE%" (
    "%PACKAGED_CORE%" --tui %*
    exit /b %ERRORLEVEL%
)
where uv >nul 2>nul
if %ERRORLEVEL% equ 0 (
    uv run python src/main.py %*
) else (
    python src/main.py %*
)

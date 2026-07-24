@echo off
setlocal
set "SCRIPT_DIR=%~dp0"
rem Refuse to run from a git clone or anywhere that is not a real install folder.
if not exist "%SCRIPT_DIR%Sky-Auto-Player.exe" (
    if not exist "%SCRIPT_DIR%Sky-Player.exe" (
        echo [!] updater.bat must live next to Sky-Auto-Player.exe or Sky-Player.exe.
        echo     Run it from the install folder, not a git clone.
        exit /b 1
    )
)
set "PS1=%SCRIPT_DIR%installer\updater.ps1"
if not exist "%PS1%" (
    echo [!] Missing: %PS1%
    exit /b 1
)
set "PS_CMD=powershell"
where pwsh >nul 2>nul
if %errorlevel%==0 set "PS_CMD=pwsh"

%PS_CMD% -NoProfile -ExecutionPolicy Bypass -File "%PS1%" %*
exit /b %errorlevel%

@echo off
echo Running Launcher.bat
echo [Dependencies] Looking for dependencies
setlocal enabledelayedexpansion


set nodejs=node
set rust=cargo
set gitwd=git

where %nodejs% >nul 2>nul
if %errorlevel% equ 0 (
    echo [Dependencies] %nodejs% is installed.
) else (
    echo [Dependencies] Node.js Not installed
    echo [Dependencies] Please install Node.js LTS here.
    echo [Dependencies] https://nodejs.org/en
)

where %rust% >nul 2>nul
if %errorlevel% equ 0 (
    echo [Dependencies] %rust% is installed.
) else (
    echo [Dependencies] Rust not installed.
    echo [Dependencies] Please install Rust here.
    echo [Dependencies] https://www.rust-lang.org/tools/install
)

where %gitwd% >nul 2>nul
if %errorlevel% equ 0 (
    echo [Dependencies] %gitwd% is installed.
) else (
    echo [Dependencies] Git not installed (Optional).
    echo [Dependencies] You can install Git here.
    echo [Dependencies] https://git-scm.com/download/win.
)


echo [Launcher] Loading include_ui
call client-backend\include_ui.bat

echo [Updater] Checking for updates
git status | find "Your branch is up to date with 'origin/main'." > nul
if errorlevel 1 (
    echo [Updater] Repository is out of date. Updating...

    REM Check for local changes
    git diff-index --quiet HEAD --
    if errorlevel 1 (
        echo [Updater] Error: Your local changes would be overwritten by pull.
        echo [Updater] Please commit, stash, or discard your changes before updating.
        echo [Updater] Bypassing update.
        REM Set custom error level (e.g., 1)
    )

    git pull origin main
    echo [Launcher] Launching Mega Anti-Cheat
    cargo run

) else (
    echo [Launcher] Repository is up to date.
    echo [Launcher] Launching Mega Anti-Cheat
    cargo run

)

pause
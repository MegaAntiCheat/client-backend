@echo off

echo Running Launcher.bat
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
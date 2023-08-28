@echo off
setlocal enabledelayedexpansion

set "UPDATE_FLAG=0"

echo "Checking for UI repository"
if exist ui\last_commit_hash.txt (
    echo "Reading existing commit hash"
    set /p LAST_COMMIT_HASH=<ui\last_commit_hash.txt
) else (
    echo "last_commit_hash.txt not found, setting to empty"
    set LAST_COMMIT_HASH=""
)

echo "Fetching latest commit hash from GitHub API"
for /f "tokens=2 delims=:" %%i in ('curl -s https://api.github.com/repos/MegaAntiCheat/MegaAntiCheat-UI/git/refs/heads/main ^| findstr /c:"\"sha\""') do set GITHUB_HASH=%%i
set GITHUB_HASH=!GITHUB_HASH:~2,40!

if "%LAST_COMMIT_HASH%"=="%GITHUB_HASH%" (
    echo "No updates to the repository."
    set "UPDATE_FLAG=1"
)

if "!UPDATE_FLAG!"=="0" (
    echo "Removing old files"
    del /s /q ui\*
    copy NUL ui\.gitkeep
    del /s /q ui_temp\*
    rmdir /s /q ui_temp

    echo "Cloning UI repository"
    git clone --filter=tree:0 https://github.com/MegaAntiCheat/MegaAntiCheat-UI ui_temp

    echo "Saving last commit hash"
    cd ui_temp
    git rev-parse HEAD > ../ui/last_commit_hash.txt

    echo "Building UI"
    call npm install -g pnpm
    call npm exec pnpm i
    call npm exec pnpm run build

    robocopy /S dist ../ui

    echo "Removing temp files..."
    cd ..
    del /s /q ui_temp\* 1>NUL
    rmdir /s /q ui_temp
)

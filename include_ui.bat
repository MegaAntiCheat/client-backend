@echo off

echo "Removing old files"
del /s /q ui\*
copy NUL ui\.gitkeep
del /s /q ui_temp\*
rmdir /s /q ui_temp

echo "Cloning UI repository"
git clone https://github.com/MegaAntiCheat/MegaAntiCheat-UI ui_temp

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

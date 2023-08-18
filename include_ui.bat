@echo off

echo "Removing old files"
del /s /q ui\*
copy NUL ui\.gitkeep
del /s /q ui_temp\*
rmdir /s /q ui_temp

echo "Cloning and building UI"
git clone https://github.com/MegaAntiCheat/MegaAntiCheat-UI ui_temp
cd ui_temp
call npm install -g pnpm
call npm exec pnpm i
call npm exec pnpm run build

robocopy /S dist ../ui

echo "Removing temp files..."
cd ..
del /s /q ui_temp\*
rmdir /s /q ui_temp
@echo off

echo Running first.bat...
call client-backend\include_ui.bat

echo Checking for updates
git remote show origin

echo Running cargo build...
cargo run

pause
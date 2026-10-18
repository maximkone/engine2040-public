@echo off
rem Double-click to join a friend's two-player arena match. See docs\DEMO.md "Two players".
rem You are slot 1. The host's console prints the address and the invite code: paste them when asked,
rem or pass them: play-arena-join.bat 192.168.1.20:7043 E2040NET-v1-...-duel-spear
rem The code carries the match (seed, length, fighters, spear), so no other flags are needed.
cd /d "%~dp0"
if not exist "target\release\arena.exe" (
  echo target\release\arena.exe is missing. Build it once with:
  echo   cargo build --release -p engine-game --features app --bin arena
  pause
  exit /b 1
)
set "ADDR=%~1"
set "CODE=%~2"
if "%ADDR%"=="" set /p "ADDR=Host address (ip:port as the host printed it): "
if "%CODE%"=="" set /p "CODE=Invite code: "
echo engine2040 arena guest: WASD move, F attack, Q throw spear, Shift block, Space dodge, Esc pauses both of you
"target\release\arena.exe" --scene arena --join %ADDR% %CODE%
if errorlevel 1 (
  echo.
  echo arena stopped with an error. The log is in target\logs. Read the last lines above.
  pause
)

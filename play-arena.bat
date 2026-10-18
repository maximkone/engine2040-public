@echo off
rem Double-click to play the engine2040 arena demo on this PC. See docs\DEMO.md.
rem You (slot 0, left of the ring) fight an AI bot. Close the window or press Ctrl+C to quit.
cd /d "%~dp0"
if not exist "target\release\arena.exe" (
  echo target\release\arena.exe is missing. Build it once with:
  echo   cargo build --release -p engine-game --features app --bin arena
  pause
  exit /b 1
)
echo engine2040 arena: WASD move, F attack, Q throw spear (tether to an enemy or wall and fly), Shift block, Space dodge, Esc pause, R rematch, C free cam
echo Extra flags go after the bat name, for example: play-arena.bat --fighters 4
"target\release\arena.exe" --scene arena --fighters 2 --play --spear --ticks 100000 %*
if errorlevel 1 (
  echo.
  echo arena stopped with an error. The log is in target\logs. Read the last lines above.
  pause
)

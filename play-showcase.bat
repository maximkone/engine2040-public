@echo off
rem Double-click to watch the engine2040 showcase tour: every system the arena plays, one
rem captioned station at a time (yank, carry, wall swing, sword hits, the finisher into a
rem ragdoll). The station's name is in the window title. Plan: docs\plan\showcase.md.
cd /d "%~dp0"
if not exist "target\release\arena.exe" (
  echo target\release\arena.exe is missing. Build it once with:
  echo   cargo build --release -p engine-game --features app --bin arena
  pause
  exit /b 1
)
echo engine2040 showcase: a script plays you against a training dummy. Watch the window title for each station.
echo The grades print here at the end: showcase: yank=ok carry=ok swing=ok melee=ok finisher=ok
rem Above-normal priority: background cargo builds (other agents) cannot starve the window.
start "" /b /abovenormal /wait "target\release\arena.exe" --scene arena --fighters 2 --showcase --ticks 100000 %*
if errorlevel 1 (
  echo.
  echo arena stopped with an error. The log is in target\logs. Read the last lines above.
  pause
)

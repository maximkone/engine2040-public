@echo off
rem Double-click to host a two-player arena match on this PC. See docs\DEMO.md "Two players".
rem You are slot 0. The console prints an invite code and the exact line your friend types.
rem Extra flags go after the bat name, for example:
rem   play-arena-host.bat --relay my.relay.example:7041   (play across the internet through net_relay)
rem   play-arena-host.bat --fighters 4                    (two AI bots join the two of you)
cd /d "%~dp0"
if not exist "target\release\arena.exe" (
  echo target\release\arena.exe is missing. Build it once with:
  echo   cargo build --release -p engine-game --features app --bin arena
  pause
  exit /b 1
)
echo engine2040 arena host: WASD move, F attack, Q throw spear, Shift block, Space dodge, Esc pauses both of you
echo The first time, Windows Firewall asks about arena.exe: allow it on private networks so your friend can reach UDP port 7043.
"target\release\arena.exe" --scene arena --fighters 2 --spear --ticks 100000 --host %*
if errorlevel 1 (
  echo.
  echo arena stopped with an error. The log is in target\logs. Read the last lines above.
  pause
)

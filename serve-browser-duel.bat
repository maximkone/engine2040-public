@echo off
rem Run the dedicated server a browser page (or a native net_join) plays a duel on. See docs\DEMO.md "Browser multiplayer".
rem   serve-browser-duel.bat HOST [more net_serve flags]
rem HOST is the name or address your other PC reaches this machine by (public IP, DNS name, or the LAN address).
rem It prints the link to open: the Pages site with ?join=HOST:7050&cert=<pinned certificate>. One 60 second match after another.
cd /d "%~dp0"
if "%~1"=="" (
  echo usage: serve-browser-duel.bat HOST [more net_serve flags]
  echo HOST is the name or address the other PC reaches this machine by.
  exit /b 1
)
if not exist "target\release\net_serve.exe" (
  echo target\release\net_serve.exe is missing. Build it once with:
  echo   cargo build --release -p engine-net --features websocket,webtransport --bin net_serve
  pause
  exit /b 1
)
echo UDP 7050 (WebTransport) must be open in the firewall and forwarded; see docs\DEMO.md. Ctrl+C stops the server.
"target\release\net_serve.exe" --ws 0.0.0.0:7050 --wt 0.0.0.0:7050 --udp 0.0.0.0:7051 --live --ticks 3600 --repeat --host %1 %2 %3 %4 %5 %6 %7 %8 %9
if errorlevel 1 (
  echo.
  echo net_serve stopped with an error. Read the last lines above.
  pause
)

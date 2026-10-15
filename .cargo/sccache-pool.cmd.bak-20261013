@echo off
rem Pool-scoped sccache dispatcher (OPT9 follow-up). Cargo runs this as:
rem wrapper ^<rustc^> ^<args...^>. Lane builds pass --target-dir
rem target/pool-N, so pool paths show up in CARGO_TARGET_DIR or in the
rem rustc args (--out-dir, --extern). Anything else (plain target/debug
rem dev builds) runs rustc directly with incremental intact. Lanes set
rem CARGO_INCREMENTAL=0 per cold command; this file never sets it.
setlocal EnableExtensions
echo %CARGO_TARGET_DIR% | findstr /C:"pool-" >nul
if not errorlevel 1 goto :cached
echo %* | findstr /C:"pool-" >nul
if not errorlevel 1 goto :cached
%*
exit /b %errorlevel%
:cached
sccache %*
exit /b %errorlevel%

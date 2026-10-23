# Build and run the integer-solver cost pair (row PX45).
#
# Float engine:  erincatto/box3d  at BOX3D_SHA
# Fixed engine:  mas-bandwidth/fixed3d at FIXED3D_SHA (Q48.16 fixed point)
#
# Why zig and not MSVC: fixed3d needs native __int128, which cl.exe does not have
# (its own Windows CI builds with `-T ClangCL`). This PC has no clang-cl, so both
# engines are built here with the same portable clang that zig ships, same
# target, same flags, scalar mode (BOX3D_DISABLE_SIMD) -- which also matches the
# fixed3d README's "2.2x float in scalar mode" claim. One compiler for both
# engines keeps the ratio about the engine, not about the compiler.
#
# Both engines build their own static library; this directory's ragdoll_cost.c
# plus the donor's own shared/determinism.c scene is then compiled against each
# library. Same scene, same step count, processes alternated so CPU contention
# from other sessions hits both engines equally.
#
# Prerequisites (no install needed, both are portable):
#   - zig: download the win64 zip from https://ziglang.org/download/ and either
#     put zig.exe on PATH or unpack it under %TEMP%\opencode\zig\zig-*\zig.exe.
#   - ninja: any on PATH; the Visual Studio copy is found automatically.
#
# Usage:
#   pwsh -NoProfile -File tools/oracle-box3d/cost/run.ps1
#   pwsh -NoProfile -File tools/oracle-box3d/cost/run.ps1 -Processes 5 -Reps 6 -Steps 500 -Zig C:\zig\zig.exe

[CmdletBinding()]
param(
    [string]$Work = (Join-Path $env:TEMP "box3d-oracle-cost"),
    [int]$Processes = 5,
    [int]$Reps = 6,
    [int]$Steps = 500,
    [string]$Zig = "",
    [string]$Ninja = ""
)

$ErrorActionPreference = "Stop"
$here = $PSScriptRoot

$BOX3D_URL = "https://github.com/erincatto/box3d"
$BOX3D_SHA = "47d7f7cc7e091142c08d11dc7d2e493c5d34f536"
$FIXED3D_URL = "https://github.com/mas-bandwidth/fixed3d"
$FIXED3D_SHA = "071a3f6de64d771ebe05ffccde3564fdb52fcdcd"
# Float baseline: box3d 47d7f7c, the commit fixed3d's own README measures
# against. Not box3d HEAD: HEAD's human ragdoll was later trimmed from 14 bones
# to 12, so HEAD and fixed3d would run different ragdoll grids (different body
# count) and the ratio would compare two scenes. The harness prints
# awake_at_create and probe sleep_step so a reader can check the two engines
# played the same grid.

function Resolve-Zig([string]$given) {
    if ($given -and (Test-Path $given)) { return (Resolve-Path $given).Path }
    $onPath = Get-Command zig -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $portable = Get-ChildItem -Path (Join-Path $env:TEMP "opencode\zig") -Filter "zig.exe" -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($portable) { return $portable.FullName }
    throw "no zig.exe found: pass -Zig <path> or put zig on PATH (see the header of this script)"
}

function Resolve-Ninja([string]$given) {
    if ($given -and (Test-Path $given)) { return (Resolve-Path $given).Path }
    $onPath = Get-Command ninja -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $vs = Join-Path $env:ProgramFiles "Microsoft Visual Studio\18\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe"
    if (Test-Path $vs) { return $vs }
    throw "no ninja.exe found: pass -Ninja <path> or put ninja on PATH"
}

New-Item -ItemType Directory -Force -Path $Work | Out-Null

$zigExe = Resolve-Zig $Zig
$ninjaExe = Resolve-Ninja $Ninja

# CMake drives a compiler through a wrapper so it sees a plain command name.
$toolchain = Join-Path $Work "toolchain"
New-Item -ItemType Directory -Force -Path $toolchain | Out-Null
function Write-Wrapper([string]$name, [string]$zigArgs, [switch]$NoTarget) {
    $path = Join-Path $toolchain "$name.cmd"
    $target = if ($NoTarget) { "" } else { " -target x86_64-windows-gnu" }
    Set-Content -Path $path -Value "@echo off`r`n`"$zigExe`" $zigArgs$target %*" -NoNewline
    return $path
}
$cc = Write-Wrapper "zig-cc" "cc"
$cxx = Write-Wrapper "zig-cxx" "c++"
# zig ar / ranlib take no -target: they only pack objects that are already built.
$ar = Write-Wrapper "zig-ar" "ar" -NoTarget
$ranlib = Write-Wrapper "zig-ranlib" "ranlib" -NoTarget

function Get-Donor([string]$name, [string]$url, [string]$sha) {
    $path = Join-Path $Work $name
    if (-not (Test-Path (Join-Path $path ".git"))) {
        git clone --quiet $url $path | Out-Null
    }
    $head = git -C $path rev-parse HEAD
    if ($head -ne $sha) {
        git -C $path fetch --quiet origin $sha | Out-Null
        git -C $path checkout --quiet $sha | Out-Null
    }
    return $path
}

function Build-Engine([string]$name, [string]$src) {
    $build = Join-Path $Work "build-$(Split-Path $src -Leaf)-zig"
    cmake -S $src -B $build -G Ninja `
        -DCMAKE_MAKE_PROGRAM="$ninjaExe" `
        -DCMAKE_C_COMPILER="$cc" -DCMAKE_CXX_COMPILER="$cxx" `
        -DCMAKE_AR="$ar" -DCMAKE_RANLIB="$ranlib" `
        -DCMAKE_BUILD_TYPE=Release `
        -DBOX3D_DISABLE_SIMD=ON `
        -DBOX3D_SAMPLES=OFF -DBOX3D_UNIT_TESTS=OFF -DBOX3D_BENCHMARKS=OFF | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "cmake configure failed for $name" }
    cmake --build $build | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "cmake build failed for $name" }
    $lib = Get-ChildItem -Path $build -Filter "libbox3d.a" -Recurse | Select-Object -First 1
    if (-not $lib) { throw "no libbox3d.a under $build" }
    return $lib.FullName
}

function Build-Harness([string]$name, [string]$src, [string]$lib, [bool]$fixedPoint) {
    $build = Join-Path $Work "cost-$name-zig"
    $fixedFlag = if ($fixedPoint) { "ON" } else { "OFF" }
    cmake -S $here -B $build -G Ninja `
        -DCMAKE_MAKE_PROGRAM="$ninjaExe" `
        -DCMAKE_C_COMPILER="$cc" -DCMAKE_CXX_COMPILER="$cxx" `
        -DCMAKE_AR="$ar" -DCMAKE_RANLIB="$ranlib" `
        -DCMAKE_BUILD_TYPE=Release `
        -DBOX3D_SRC="$src" -DBOX3D_LIB="$lib" -DBOX3D_LABEL="$name" -DBOX3D_FIXED="$fixedFlag" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "cmake configure failed for cost-$name" }
    cmake --build $build | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "cmake build failed for cost-$name" }
    $exe = Get-ChildItem -Path $build -Filter "ragdoll_cost.exe" -Recurse | Select-Object -First 1
    if (-not $exe) { throw "no ragdoll_cost.exe under $build" }
    return $exe.FullName
}

$floatSrc = Get-Donor "box3d-47d7f7c" $BOX3D_URL $BOX3D_SHA
$fixedSrc = Get-Donor "fixed3d" $FIXED3D_URL $FIXED3D_SHA

Write-Host "zig:      $zigExe"
Write-Host "ninja:    $ninjaExe"
Write-Host "float src: $floatSrc @ $(git -C $floatSrc rev-parse --short HEAD)"
Write-Host "fixed src: $fixedSrc @ $(git -C $fixedSrc rev-parse --short HEAD)"

$floatLib = Build-Engine "float" $floatSrc
$fixedLib = Build-Engine "fixed" $fixedSrc
$floatExe = Build-Harness "float" $floatSrc $floatLib $false
$fixedExe = Build-Harness "fixed" $fixedSrc $fixedLib $true

Write-Host "float exe: $floatExe"
Write-Host "fixed exe: $fixedExe"

$log = Join-Path $Work "cost-runs.txt"
if (Test-Path $log) { Remove-Item $log }

for ($i = 1; $i -le $Processes; $i++) {
    & $floatExe --steps $Steps --reps $Reps | Tee-Object -FilePath $log -Append
    & $fixedExe --steps $Steps --reps $Reps | Tee-Object -FilePath $log -Append
}

function Get-Median([double[]]$values) {
    $sorted = $values | Sort-Object
    $count = $sorted.Count
    if ($count -eq 0) { return 0.0 }
    if ($count % 2 -eq 1) { return $sorted[[int](($count - 1) / 2)] }
    return ($sorted[$count / 2 - 1] + $sorted[$count / 2]) / 2.0
}

$floatCpu = @(); $fixedCpu = @(); $floatWall = @(); $fixedWall = @()
foreach ($line in Get-Content $log) {
    if ($line -notmatch "engine=(\w+).* cpu_ms=([0-9.]+) wall_ms=([0-9.]+)") { continue }
    $engine = $Matches[1]
    $cpu = [double]$Matches[2]
    $wall = [double]$Matches[3]
    if ($engine -eq "float") { $floatCpu += $cpu; $floatWall += $wall }
    else { $fixedCpu += $cpu; $fixedWall += $wall }
}

$fc = Get-Median $floatCpu
$xc = Get-Median $fixedCpu
$fw = Get-Median $floatWall
$xw = Get-Median $fixedWall

""
"engine  runs  cpu_median_ms  cpu_min_ms  wall_median_ms"
"float   {0,4}  {1,12:N2}  {2,10:N2}  {3,14:N2}" -f $floatCpu.Count, $fc, ($floatCpu | Measure-Object -Minimum).Minimum, $fw
"fixed   {0,4}  {1,12:N2}  {2,10:N2}  {3,14:N2}" -f $fixedCpu.Count, $xc, ($fixedCpu | Measure-Object -Minimum).Minimum, $xw
""
"factor_cpu_median  = {0:N2}x" -f ($xc / $fc)
"factor_cpu_min     = {0:N2}x" -f ((($fixedCpu | Measure-Object -Minimum).Minimum) / (($floatCpu | Measure-Object -Minimum).Minimum))
"factor_wall_median = {0:N2}x (shared PC: other sessions steal wall time)" -f ($xw / $fw)
"log                = $log"
"steps={0} reps={1} processes={2}" -f $Steps, $Reps, $Processes
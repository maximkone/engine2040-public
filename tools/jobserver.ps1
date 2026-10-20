# jobserver: one machine-wide pool of compile slots shared by every cargo on this PC.
#
# Cargo and rustc speak the GNU make jobserver protocol. On Windows the pool is a
# named semaphore: a cargo started with
#   CARGO_MAKEFLAGS=-j --jobserver-auth=<name>
# opens it (jobserver crate, windows.rs: OpenSemaphoreA) and takes one slot per
# extra rustc instead of starting its own -j pool. Many sessions then share
# <Tokens> slots plus one implicit slot each, instead of 12 rustc jobs each.
# No build waits on a lock: every build runs, they just share the CPU.
# Measured 2026-10-21 on a full box: 1 slot, 2 rustc, 591 s; own -j, 10 rustc, 588 s.
#
# Usage (pwsh):
#   pwsh -NoProfile -File tools/jobserver.ps1 install     # permanent: user env var + start at logon + start now
#   pwsh -NoProfile -File tools/jobserver.ps1 uninstall   # undo all of install and stop the holder
#   pwsh -NoProfile -File tools/jobserver.ps1 start [-Tokens 8]   # hold the pool (runs until stopped)
#   pwsh -NoProfile -File tools/jobserver.ps1 status      # is the pool there, how many free
#   pwsh -NoProfile -File tools/jobserver.ps1 env         # print the CARGO_MAKEFLAGS line
# A cargo whose CARGO_MAKEFLAGS names a missing pool falls back to its own -j
# (checked 2026-10-21), so a stopped holder never breaks a build.
param(
    [Parameter(Position = 0)][ValidateSet('start', 'status', 'env', 'install', 'uninstall')][string]$Action = 'status',
    # 8 of 12 cores: each live build also keeps its one implicit slot, so 3-4 live builds land
    # near 12 jobs on 12 cores.
    [int]$Tokens = 8,
    [string]$Name = 'engine2040-jobserver'
)

$flags = "-j --jobserver-auth=$Name"
$link = Join-Path ([Environment]::GetFolderPath('Startup')) "$Name.lnk"
$holders = {
    Get-CimInstance Win32_Process -Filter "name='pwsh.exe' or name='powershell.exe'" |
        Where-Object { $_.ProcessId -ne $PID -and $_.CommandLine -match 'jobserver\.ps1' -and $_.CommandLine -match '\sstart(\s|$)' }
}

switch ($Action) {
    'env' {
        Write-Output "CARGO_MAKEFLAGS=$flags"
    }
    'status' {
        $sem = $null
        if ([System.Threading.Semaphore]::TryOpenExisting($Name, [ref]$sem)) {
            # Count the free slots by taking them all without waiting, then give them back.
            $free = 0
            while ($sem.WaitOne(0)) { $free++ }
            if ($free -gt 0) { [void]$sem.Release($free) }
            Write-Output "jobserver ${Name}: up, $free slots free"
            $sem.Dispose()
        } else {
            Write-Output "jobserver ${Name}: down"
            exit 1
        }
    }
    'start' {
        # One holder per pool. The mutex is the proof of a live holder: a dead holder's mutex
        # comes back abandoned, which still hands it over.
        $mutex = [System.Threading.Mutex]::new($false, "$Name-holder")
        try { $mine = $mutex.WaitOne(0) } catch [System.Threading.AbandonedMutexException] { $mine = $true }
        if (-not $mine) {
            Write-Output "jobserver ${Name}: a holder is already running"
            exit 0
        }
        # The max sits above the slot count so a slot given back twice (a killed build's cleanup
        # racing the balance below) never makes a cargo's release fail; the balance takes it back.
        # If the pool already exists (the old holder died while builds still had it open), this
        # opens the same one and keeps it.
        $created = $false
        $sem = [System.Threading.Semaphore]::new($Tokens, $Tokens * 4, $Name, [ref]$created)
        $how = if ($created) { 'up' } else { 'adopted' }
        Write-Output "jobserver ${Name}: $how with $Tokens slots; builds join with CARGO_MAKEFLAGS=$flags"
        try {
            while ($true) {
                Start-Sleep -Seconds 30
                # Free slots never exceed $Tokens: an adopted pool or a slot given back twice made
                # extra ones, so drain the free slots and give back at most $Tokens. A build killed
                # mid-job never gives its slot back, so slots also leak: when nothing compiles (no
                # rustc, no build script) every slot must be free, so give back exactly $Tokens.
                $idle = @(Get-Process -Name 'rustc', 'build-script-*' -ErrorAction SilentlyContinue).Count -eq 0
                $free = 0
                while ($sem.WaitOne(0)) { $free++ }
                $back = if ($idle) { $Tokens } else { [Math]::Min($free, $Tokens) }
                # One at a time: an adopted pool may have a smaller max than $Tokens.
                for ($i = 0; $i -lt $back; $i++) {
                    try { [void]$sem.Release(1) } catch [System.Threading.SemaphoreFullException] { break }
                }
                if ($back -ne $free) { Write-Output "jobserver ${Name}: $free free, set to $back" }
            }
        } finally {
            $sem.Dispose()
            $mutex.ReleaseMutex()
        }
    }
    'install' {
        # Every process started after this (new shells, restarted apps) joins the pool. Running
        # ones keep the env they started with until they restart.
        [Environment]::SetEnvironmentVariable('CARGO_MAKEFLAGS', $flags, 'User')
        $pwsh = (Get-Command pwsh).Source
        $shell = New-Object -ComObject WScript.Shell
        $lnk = $shell.CreateShortcut($link)
        $lnk.TargetPath = $pwsh
        $lnk.Arguments = "-NoProfile -WindowStyle Hidden -File `"$PSCommandPath`" start -Tokens $Tokens -Name $Name"
        $lnk.WindowStyle = 7
        $lnk.Save()
        if (-not @(& $holders).Count) {
            Start-Process $pwsh -WindowStyle Hidden -ArgumentList '-NoProfile', '-File', "`"$PSCommandPath`"", 'start', '-Tokens', $Tokens, '-Name', $Name
        }
        Write-Output "jobserver ${Name}: installed (user env CARGO_MAKEFLAGS=$flags, starts at logon: $link)"
    }
    'uninstall' {
        [Environment]::SetEnvironmentVariable('CARGO_MAKEFLAGS', $null, 'User')
        Remove-Item -LiteralPath $link -ErrorAction SilentlyContinue
        & $holders | ForEach-Object { Stop-Process -Id $_.ProcessId -Confirm:$false -ErrorAction SilentlyContinue }
        Write-Output "jobserver ${Name}: uninstalled (env var, logon item and holder gone)"
    }
}

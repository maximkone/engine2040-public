# jobserver: one machine-wide pool of compile slots shared by every cargo on this PC.
#
# Cargo and rustc speak the GNU make jobserver protocol. On Windows the pool is a
# named semaphore: a cargo started with
#   CARGO_MAKEFLAGS=-j --jobserver-auth=<name>
# opens it (jobserver crate, windows.rs: OpenSemaphoreA) and takes one slot per
# extra rustc instead of starting its own -j pool. Many sessions then share
# <Tokens> slots plus one implicit slot each, instead of 12 rustc jobs each.
# No build waits on a lock: every build runs, they just share the CPU.
#
# Usage (pwsh):
#   pwsh -NoProfile -File tools/jobserver.ps1 start [-Tokens 10]   # hold the pool (runs until stopped)
#   pwsh -NoProfile -File tools/jobserver.ps1 status               # is the pool there, how many free
#   pwsh -NoProfile -File tools/jobserver.ps1 env                  # print the CARGO_MAKEFLAGS line
# A cargo whose CARGO_MAKEFLAGS names a missing pool warns and falls back to its
# own -j, so a stopped holder never breaks a build.
param(
    [Parameter(Position = 0)][ValidateSet('start', 'status', 'env')][string]$Action = 'status',
    [int]$Tokens = 10,
    [string]$Name = 'engine2040-jobserver'
)

$flags = "-j --jobserver-auth=$Name"
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
        $created = $false
        # The max sits above the slot count so a slot given back twice (a killed build's cleanup
        # racing the balance below) never makes a cargo's release fail; the balance takes it back.
        $sem = [System.Threading.Semaphore]::new($Tokens, $Tokens * 4, $Name, [ref]$created)
        if (-not $created) {
            Write-Output "jobserver ${Name}: already up, not starting a second holder"
            exit 0
        }
        Write-Output "jobserver ${Name}: up with $Tokens slots; builds join with CARGO_MAKEFLAGS=$flags"
        try {
            while ($true) {
                Start-Sleep -Seconds 30
                # A build killed mid-job never gives its slot back, so slots leak over a day. When
                # nothing compiles (no rustc, no build script) every slot must be free: drain the
                # free ones and give back exactly $Tokens.
                if (@(Get-Process -Name 'rustc', 'build-script-*' -ErrorAction SilentlyContinue).Count -gt 0) { continue }
                $free = 0
                while ($sem.WaitOne(0)) { $free++ }
                [void]$sem.Release($Tokens)
                if ($free -ne $Tokens) { Write-Output "jobserver ${Name}: idle with $free free, reset to $Tokens" }
            }
        } finally {
            $sem.Dispose()
        }
    }
}

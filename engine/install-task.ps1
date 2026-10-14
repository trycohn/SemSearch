<#
.SYNOPSIS
    Installs the SemSearch engine for elevated use and registers the "SemSearch Engine" task.

.DESCRIPTION
    An elevated task must not run an executable from the user-writable project folder
    (any medium-integrity process could replace it). So this script (self-elevating via UAC):
      1. copies <root>\bin\semsearch-engine.exe to "<Program Files>\SemSearch\semsearch-engine.exe"
         (the folder inherits Program Files ACLs: administrators write, users read/execute);
      2. registers the scheduled task "SemSearch Engine" that runs THAT copy with
         --root "<project root>" at logon of the user, RunLevel Highest, no time limit,
         restart on failure;
      3. asks a running (e.g. non-elevated, backend-started) engine to shut down and starts
         the task, retrying until the elevated engine owns \\.\pipe\semsearch-engine.
    The elevated engine logs to "<Program Files>\SemSearch\logs\engine.log".

.PARAMETER User
    Account to run the task for (default: the user who started the script).

.PARAMETER NoPause
    Do not wait for Enter before closing the elevated window.
#>
param(
    [string]$User = "",
    [switch]$NoPause,
    [switch]$Elevated
)

$ErrorActionPreference = 'Stop'
$TaskName = 'SemSearch Engine'
$PipeName = 'semsearch-engine'

function Test-Admin {
    $p = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Invoke-Engine([string]$Json, [int]$TimeoutMs = 1500) {
    # One request/response over the engine pipe; returns the response line or $null.
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', $PipeName, [System.IO.Pipes.PipeDirection]::InOut)
    try {
        $pipe.Connect($TimeoutMs)
        $enc = New-Object System.Text.UTF8Encoding($false)
        $w = New-Object System.IO.StreamWriter($pipe, $enc)
        $w.AutoFlush = $true
        $r = New-Object System.IO.StreamReader($pipe, $enc)
        $w.WriteLine($Json)
        return $r.ReadLine()
    } catch {
        return $null
    } finally {
        $pipe.Dispose()
    }
}

function Get-ProgramFiles64 {
    if ($env:ProgramW6432) { return $env:ProgramW6432 }
    return $env:ProgramFiles
}

function Wait-ProcessGone([string]$ExePath, [int]$Seconds = 10) {
    for ($i = 0; $i -lt ($Seconds * 4); $i++) {
        $procs = @(Get-Process -Name 'semsearch-engine' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($_.Path -ieq $ExePath) })
        if ($procs.Count -eq 0) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

if (-not $User) { $User = [Security.Principal.WindowsIdentity]::GetCurrent().Name }

if (-not (Test-Admin)) {
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-User', "`"$User`"", '-Elevated')
    if ($NoPause) { $argList += '-NoPause' }
    try {
        $p = Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList $argList -Wait -PassThru
        exit $p.ExitCode
    } catch {
        Write-Error "Elevation was cancelled or failed: $_"
        exit 1
    }
}

$code = 0
try {
    $Root = (Split-Path -Parent $PSScriptRoot).TrimEnd('\')
    $Src = Join-Path $Root 'bin\semsearch-engine.exe'
    if (-not (Test-Path -LiteralPath $Src -PathType Leaf)) { throw "Engine not found: $Src" }
    if (-not (Test-Path -LiteralPath (Join-Path $Root 'config.json') -PathType Leaf)) {
        Write-Warning "config.json not found in $Root (the engine will index all fixed NTFS drives)"
    }
    $PF = Get-ProgramFiles64
    if (-not $PF -or -not (Test-Path -LiteralPath $PF -PathType Container)) { throw "Program Files folder not found" }
    $InstallDir = Join-Path $PF 'SemSearch'
    $Exe = Join-Path $InstallDir 'semsearch-engine.exe'
    Write-Host "Project root: $Root"
    Write-Host "Install to:   $Exe"
    Write-Host "User:         $User"

    # Stop the task and any running engine (it may hold the pipe or lock the installed exe).
    if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    }
    $null = Invoke-Engine '{"id":1,"op":"shutdown"}'
    if (-not (Wait-ProcessGone $Exe 10)) {
        Get-Process -Name 'semsearch-engine' -ErrorAction SilentlyContinue | Where-Object { $_.Path -ieq $Exe } | Stop-Process -Force
        $null = Wait-ProcessGone $Exe 5
    }

    # Copy into Program Files; reset the folder ACL to the inherited one (users: read/execute).
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    & icacls.exe $InstallDir /reset /T /C /Q | Out-Null
    Copy-Item -LiteralPath $Src -Destination $Exe -Force
    $h1 = (Get-FileHash -LiteralPath $Src -Algorithm SHA256).Hash
    $h2 = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
    if ($h1 -ne $h2) { throw "Copy verification failed ($h1 vs $h2)" }
    Write-Host "Copied, SHA256 $h2"

    $arg = "--root `"$Root`""
    $action = New-ScheduledTaskAction -Execute $Exe -Argument $arg -WorkingDirectory $InstallDir
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
    $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet `
        -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -RestartCount 999 `
        -RestartInterval (New-TimeSpan -Minutes 1) `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -DontStopOnIdleEnd `
        -StartWhenAvailable `
        -MultipleInstances IgnoreNew `
        -Priority 7
    $desc = 'SemSearch file-name engine (MFT + USN journal index, named pipe \\.\pipe\semsearch-engine).'
    Register-ScheduledTask -TaskName $TaskName -Description $desc -Action $action -Trigger $trigger `
        -Principal $principal -Settings $settings -Force | Out-Null
    Write-Host "Task '$TaskName' registered: `"$Exe`" $arg"

    # The backend may respawn a non-elevated engine at any moment; retry until the elevated one owns the pipe.
    $ok = $false
    for ($attempt = 1; $attempt -le 5 -and -not $ok; $attempt++) {
        $ping = Invoke-Engine '{"id":2,"op":"ping"}'
        if ($ping -and $ping -notmatch '"elevated":\s*true') {
            Write-Host "Non-elevated engine is running ($ping) -> asking it to shut down"
            $null = Invoke-Engine '{"id":3,"op":"shutdown"}'
            Start-Sleep -Milliseconds 500
        }
        Start-ScheduledTask -TaskName $TaskName
        for ($i = 0; $i -lt 20; $i++) {
            Start-Sleep -Milliseconds 500
            $ping = Invoke-Engine '{"id":4,"op":"ping"}'
            if ($ping -match '"elevated":\s*true') { $ok = $true; break }
            if ($ping) { break }  # someone else answered; retry
        }
    }
    if ($ok) {
        Write-Host "Elevated engine answers: $ping"
        Write-Host "Logs: $InstallDir\logs\engine.log"
    } else {
        Write-Warning "Elevated engine did not take over the pipe; see $InstallDir\logs\engine.log"
        $code = 2
    }
} catch {
    Write-Error $_
    $code = 1
}

if ($Elevated -and -not $NoPause) { Read-Host 'Press Enter to close' | Out-Null }
exit $code

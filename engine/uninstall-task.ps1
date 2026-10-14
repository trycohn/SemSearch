<#
.SYNOPSIS
    Removes the "SemSearch Engine" scheduled task, stops the engine and deletes
    "<Program Files>\SemSearch" (the installed engine copy and its logs).

.DESCRIPTION
    Self-elevates via UAC when started without administrator rights.
    The project folder (bin\, data\) is not touched; the backend may start a
    non-elevated engine from bin\ again afterwards.

.PARAMETER NoPause
    Do not wait for Enter before closing the elevated window.
#>
param(
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

function Invoke-Engine([string]$Json, [int]$TimeoutMs = 1000) {
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

if (-not (Test-Admin)) {
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Elevated')
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
    $PF = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
    if (-not $PF -or -not (Test-Path -LiteralPath $PF -PathType Container)) { throw "Program Files folder not found" }
    $InstallDir = Join-Path $PF 'SemSearch'
    $Exe = Join-Path $InstallDir 'semsearch-engine.exe'

    $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($task) {
        Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        Write-Host "Task '$TaskName' removed."
    } else {
        Write-Host "Task '$TaskName' is not registered."
    }

    # Ask the running engine to exit (it never changes anything on disk); make sure the installed copy is gone.
    if (Invoke-Engine '{"id":1,"op":"shutdown"}') { Write-Host 'Running engine asked to shut down.' }
    for ($i = 0; $i -lt 40; $i++) {
        $procs = @(Get-Process -Name 'semsearch-engine' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($_.Path -ieq $Exe) })
        if ($procs.Count -eq 0) { break }
        if ($i -eq 20) { $procs | Stop-Process -Force }
        Start-Sleep -Milliseconds 250
    }

    # Remove only "<Program Files>\SemSearch".
    if ((Split-Path -Leaf $InstallDir) -eq 'SemSearch' -and (Split-Path -Parent $InstallDir) -ieq $PF -and (Test-Path -LiteralPath $InstallDir -PathType Container)) {
        Remove-Item -LiteralPath $InstallDir -Recurse -Force
        Write-Host "Removed $InstallDir"
    } else {
        Write-Host "$InstallDir does not exist."
    }
} catch {
    Write-Error $_
    $code = 1
}

if ($Elevated -and -not $NoPause) { Read-Host 'Press Enter to close' | Out-Null }
exit $code

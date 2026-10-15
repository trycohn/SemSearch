@echo off
rem Starts uninstall-task.ps1: removes the "SemSearch Engine" task and the engine copy installed by install-task.
rem -ExecutionPolicy Bypass applies to this one script only: it works under the default Restricted policy
rem and for a script from a downloaded ZIP. Full path: does not depend on PATH; no chcp - the file is ASCII.
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall-task.ps1" %*
exit /b %errorlevel%

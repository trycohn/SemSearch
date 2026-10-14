@echo off
chcp 65001 >nul
rem Удаление задачи Планировщика и копии движка из Program Files, см. uninstall-task.ps1.
rem -ExecutionPolicy Bypass: скрипт запускается и при политике по умолчанию (Restricted), и из скачанного ZIP.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall-task.ps1" %*
exit /b %errorlevel%

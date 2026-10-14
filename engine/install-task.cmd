@echo off
chcp 65001 >nul
rem Установка движка задачей Планировщика (режим MFT), см. install-task.ps1.
rem -ExecutionPolicy Bypass: скрипт запускается и при политике по умолчанию (Restricted), и из скачанного ZIP.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-task.ps1" %*
exit /b %errorlevel%

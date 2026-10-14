@echo off
chcp 65001 >nul
cd /d "%~dp0"
if exist SemSearch.exe (
    start "" SemSearch.exe %*
    exit /b
)
if not exist .venv\Scripts\pythonw.exe (echo Сначала запустите setup.bat & pause & exit /b 1)
start "" .venv\Scripts\pythonw.exe server.py %*

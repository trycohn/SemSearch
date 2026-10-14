@echo off
chcp 65001 >nul
cd /d "%~dp0"
where py >nul 2>nul || (echo Нужен Python 3.11+: https://www.python.org/downloads/ & pause & exit /b 1)
if not exist .venv\Scripts\python.exe (
    py -3.11 -m venv .venv 2>nul || py -3 -m venv .venv
)
.venv\Scripts\python.exe -m pip install -U pip
.venv\Scripts\python.exe -m pip install torch torchvision --index-url https://download.pytorch.org/whl/cu128
.venv\Scripts\python.exe -m pip install -r requirements.txt
echo.
echo Готово. Запуск: start.bat  (при первом запуске скачается модель ~1.5 ГБ)
pause

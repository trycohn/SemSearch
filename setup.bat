@echo off
chcp 65001 >nul
cd /d "%~dp0"
rem Python 3.11+: py -3.11, затем py -3, затем python из PATH (версию проверяем, заглушка Microsoft Store не пройдёт)
set "PY="
for %%P in ("py -3.11" "py -3" "python") do if not defined PY (
    %%~P -c "import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)" >nul 2>nul && set "PY=%%~P"
)
if not defined PY (
    echo Не найден Python 3.11 или новее. Установите его с https://www.python.org/downloads/
    echo ^(галочка «Add python.exe to PATH»^) и запустите setup.bat ещё раз.
    pause
    exit /b 1
)
if not exist .venv\Scripts\python.exe (
    %PY% -m venv .venv || goto :fail
)
.venv\Scripts\python.exe -m pip install -U pip || goto :fail
rem PyTorch: сборка с CUDA, если есть драйвер NVIDIA (nvidia-smi), иначе — для процессора
set "TORCH_INDEX=https://download.pytorch.org/whl/cpu"
where nvidia-smi >nul 2>nul && set "TORCH_INDEX=https://download.pytorch.org/whl/cu128"
echo PyTorch: %TORCH_INDEX%
.venv\Scripts\python.exe -m pip install torch==2.11.0 torchvision==0.26.0 --index-url %TORCH_INDEX% || goto :fail
.venv\Scripts\python.exe -m pip install -r requirements.txt || goto :fail
echo.
echo Готово. Запуск: start.bat  (при первом запуске скачается модель ~1.5 ГБ)
pause
exit /b 0

:fail
echo.
echo Установка прервана — причина в сообщениях выше. Исправьте её и запустите setup.bat ещё раз.
pause
exit /b 1

@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"
rem Сборка Rust-частей: engine -> bin\semsearch-engine.exe, shell -> SemSearch.exe.
rem Каждая часть собирается из своей папки: Cargo читает .cargo\config.toml от текущей папки вверх
rem (у движка там статический CRT, и exe не требует VC++ Redistributable).
where cargo >nul 2>nul || (echo Нужен Rust: https://rustup.rs & exit /b 1)
if defined RUSTFLAGS echo Внимание: задан RUSTFLAGS, он заменяет флаги из engine\.cargo\config.toml — движку понадобится VC++ Redistributable.

echo === Движок имён: engine ===
pushd engine || exit /b 1
cargo build --release --locked
if errorlevel 1 (
    popd
    echo Сборка движка не удалась.
    exit /b 1
)
popd

echo === Оболочка: shell ===
pushd shell || exit /b 1
cargo build --release --locked
if errorlevel 1 (
    popd
    echo Сборка оболочки не удалась.
    exit /b 1
)
popd

if not exist bin mkdir bin || exit /b 1
call :put engine\target\release\semsearch-engine.exe bin\semsearch-engine.exe || exit /b 1
call :put shell\target\release\SemSearch.exe SemSearch.exe || exit /b 1
echo.
echo Готово: bin\semsearch-engine.exe и SemSearch.exe обновлены.
schtasks /query /tn "SemSearch Engine" >nul 2>nul && echo Движок установлен задачей Планировщика — обновите его копию: engine\install-task.cmd
exit /b 0

rem Копирование %1 в %2. Запущенный exe заменить нельзя: проверяем, что %2 открывается на запись.
:put
if exist "%~2" (
    2>nul (>>"%~2" (call )) || (
        echo Не удалось заменить %~2: файл занят — похоже, SemSearch запущен. Закройте SemSearch через меню в трее → «Выход» и запустите build.cmd ещё раз.
        exit /b 1
    )
)
copy /y "%~1" "%~2" >nul || (
    echo Не удалось скопировать %~1 в %~2.
    exit /b 1
)
exit /b 0

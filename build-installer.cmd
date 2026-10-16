@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"
rem Сборка установщика: dist\SemSearch-Setup-<версия>.exe (docs/INSTALLER.md, раздел «Сборка»).
rem 1) build.cmd: движок и оболочка; 2) cargo build в installer\: установщик и упаковщик pack.exe;
rem 3) копия установщика -> uninstall.exe; подпись SemSearch.exe, bin\semsearch-engine.exe и uninstall.exe;
rem 4) Python по installer\python.txt в installer\.cache\ (с докачкой и проверкой sha256);
rem 5) файлы программы по installer\payload.txt + Python + uninstall.exe -> zip;
rem 6) zip дописывается к неподписанному exe установщика, затем подписывается готовый exe.
rem Каждый файл подписывается один раз. Подпись необязательна, её задают переменные окружения:
rem   SEMSEARCH_SIGN        аргументы signtool, выбирающие сертификат:
rem                         /f файл.pfx /p пароль  |  /sha1 <отпечаток> (хранилище или токен)  |
rem                         /dlib <Azure.CodeSigning.Dlib.dll> /dmdf <metadata.json> (Azure Trusted Signing)
rem   SEMSEARCH_SIGNTOOL    путь к signtool.exe; по умолчанию — самый новый из Windows Kits 10 (bin\<версия>\x64)
rem   SEMSEARCH_SIGN_TS     сервер меток времени RFC 3161; по умолчанию http://timestamp.digicert.com
rem   SEMSEARCH_SIGN_TEST=1 тестовый самоподписанный сертификат: недоверенный корень допустим,
rem                         но не повреждённый хэш и не отсутствие подписи
rem Этапы — для подписи вне этой машины (SignPath в .github\workflows\release.yml), между ними файлы подписывают:
rem   build-installer.cmd                всё сразу, шаги 1-6;
rem   build-installer.cmd inner          шаги 1-3 и остановка: три exe для подписи — в installer\target\inner\
rem                                      (SemSearch.exe, bin\semsearch-engine.exe, uninstall.exe);
rem   build-installer.cmd pack [папка]   шаги 4-6 из собранного этапом inner; папка (полный путь или от корня
rem                                      репозитория) — те же три файла с теми же путями, уже подписанные: они сначала
rem                                      встают на свои места. Готовый exe подписывают после этого этапа.
rem SEMSEARCH_SIGN действует и на этапах: inner подписывает три файла, pack — готовый exe.
set "STAGE=%~1"
if "%STAGE%"=="" goto :stage_ok
if /i "%STAGE%"=="inner" goto :stage_ok
if /i "%STAGE%"=="pack" goto :stage_ok
echo Неизвестный этап «%STAGE%». Запуск: build-installer.cmd [inner ^| pack [папка с подписанными файлами]]
exit /b 1
:stage_ok
if /i "%STAGE%"=="pack" goto :tools_ok
where cargo >nul 2>nul || (echo Нужен Rust: https://rustup.rs & exit /b 1)
if defined RUSTFLAGS echo Внимание: задан RUSTFLAGS, он заменяет флаги из installer\.cargo\config.toml — установщику понадобится VC++ Redistributable.
:tools_ok

set "VER="
for /f "usebackq delims=" %%v in (`powershell -NoProfile -Command "(Get-Content -Raw -Encoding UTF8 'shell\tauri.conf.json' | ConvertFrom-Json).version"`) do set "VER=%%v"
if not defined VER (
    echo Не удалось прочитать версию из shell\tauri.conf.json.
    exit /b 1
)
echo Версия: %VER%

set "TS=%SEMSEARCH_SIGN_TS%"
if not defined TS set "TS=http://timestamp.digicert.com"
if not defined SEMSEARCH_SIGN goto :no_sign
call :find_signtool || exit /b 1
echo Подпись: "%SIGNTOOL%", метка времени %TS%
if "%SEMSEARCH_SIGN_TEST%"=="1" echo Тестовый сертификат: недоверенный корень допустим.
goto :signing_set
:no_sign
echo Подпись пропущена: SEMSEARCH_SIGN не задан — exe будут без цифровой подписи. Как подписать — docs\INSTALLER.md, раздел «Сборка».
:signing_set

set "PACK=installer\target\release\pack.exe"
set "SETUP=installer\target\release\SemSearch-Setup.exe"
set "UN=installer\target\uninstall.exe"
set "ZIP=installer\target\payload.zip"
set "INNER=installer\target\inner"
set "OUT=dist\SemSearch-Setup-%VER%.exe"
if /i "%STAGE%"=="pack" goto :stage_pack

echo === 1/6 Движок и оболочка: build.cmd ===
call "%~dp0build.cmd"
if errorlevel 1 (
    echo Установщик не собран: не удалось собрать движок или оболочку — причина выше.
    exit /b 1
)

echo === 2/6 Установщик: installer ===
if not exist installer\ui\index.html (
    echo Нет installer\ui\index.html — интерфейса установщика.
    exit /b 1
)
pushd installer || exit /b 1
cargo build --release --locked
if errorlevel 1 (
    popd
    echo Сборка установщика не удалась.
    exit /b 1
)
popd

echo === 3/6 uninstall.exe и подпись программы ===
copy /y "%SETUP%" "%UN%" >nul
if errorlevel 1 (
    echo Не удалось скопировать %SETUP% в %UN%.
    exit /b 1
)
call :sign SemSearch.exe || exit /b 1
call :sign bin\semsearch-engine.exe || exit /b 1
call :sign "%UN%" || exit /b 1
if /i not "%STAGE%"=="inner" goto :step_python

rem этап inner: три exe в одну папку, с теми же путями, что и в корне программы
if exist "%INNER%" rmdir /s /q "%INNER%"
mkdir "%INNER%\bin" || exit /b 1
call :take SemSearch.exe "%INNER%\SemSearch.exe" || exit /b 1
call :take bin\semsearch-engine.exe "%INNER%\bin\semsearch-engine.exe" || exit /b 1
call :take "%UN%" "%INNER%\uninstall.exe" || exit /b 1
echo.
echo Готово: этап inner, файлы для подписи — в %INNER%\. Дальше: build-installer.cmd pack ^<папка с подписанными^>
exit /b 0

rem этап pack: собранное этапом inner на месте; подписанные файлы из папки (если задана) встают на свои места
:stage_pack
for %%f in ("%PACK%" "%SETUP%" "%UN%" "SemSearch.exe" "bin\semsearch-engine.exe") do if not exist "%%~f" (
    echo Нет %%~f: сначала build-installer.cmd inner.
    exit /b 1
)
if "%~2"=="" goto :step_python
if not exist "%~2\" (
    echo Нет папки с подписанными файлами: "%~2"
    exit /b 1
)
echo Подписанные файлы: %~f2
call :take "%~2\SemSearch.exe" SemSearch.exe || exit /b 1
call :take "%~2\bin\semsearch-engine.exe" bin\semsearch-engine.exe || exit /b 1
call :take "%~2\uninstall.exe" "%UN%" || exit /b 1

:step_python
echo === 4/6 Python: installer\python.txt ===
"%PACK%" python .
if errorlevel 1 (
    echo Не удалось получить Python — проверьте интернет и адрес с sha256 в installer\python.txt.
    exit /b 1
)

echo === 5/6 Файлы программы: installer\payload.txt ===
"%PACK%" zip . "%ZIP%" "%UN%"
if errorlevel 1 (
    echo Не удалось упаковать файлы программы — причина выше.
    exit /b 1
)

echo === 6/6 %OUT% ===
if not exist dist mkdir dist || exit /b 1
rem полезная нагрузка — к неподписанному exe; подпись готового exe покрывает и её
"%PACK%" append "%SETUP%" "%ZIP%" "%OUT%"
if errorlevel 1 (
    echo Не удалось записать %OUT% — файл занят? Закройте запущенный установщик и повторите.
    exit /b 1
)
call :sign "%OUT%" || exit /b 1
"%PACK%" info "%OUT%"
if errorlevel 1 (
    echo Установщик не находит в %OUT% файлов программы.
    exit /b 1
)
del "%ZIP%" >nul 2>nul
echo.
if defined SEMSEARCH_SIGN (echo Готово: %OUT% — подписан.) else echo Готово: %OUT% — без подписи.
exit /b 0

rem Копия %1 в %2 для этапов inner и pack.
:take
if not exist "%~1" (
    echo Нет файла %~1
    exit /b 1
)
copy /y "%~1" "%~2" >nul || (
    echo Не удалось скопировать %~1 в %~2 — файл занят? Закройте SemSearch и повторите.
    exit /b 1
)
exit /b 0

rem ---------------------------------------------------------------- подпись

rem signtool.exe: SEMSEARCH_SIGNTOOL или самый новый из Windows Kits 10.
:find_signtool
if not defined SEMSEARCH_SIGNTOOL goto :signtool_kits
set "SIGNTOOL=%SEMSEARCH_SIGNTOOL%"
if exist "%SIGNTOOL%" exit /b 0
echo Нет файла из SEMSEARCH_SIGNTOOL: "%SIGNTOOL%"
exit /b 1
:signtool_kits
set "SIGNTOOL="
set "KITS=%ProgramFiles(x86)%\Windows Kits\10\bin"
for /f "delims=" %%d in ('dir /b /ad /o-n "%KITS%\10.*" 2^>nul') do if not defined SIGNTOOL if exist "%KITS%\%%d\x64\signtool.exe" set "SIGNTOOL=%KITS%\%%d\x64\signtool.exe"
if defined SIGNTOOL exit /b 0
echo Не найден signtool.exe: поставьте Windows SDK с компонентом Signing Tools или задайте SEMSEARCH_SIGNTOOL.
exit /b 1

rem Подписать %1 и проверить подпись (без SEMSEARCH_SIGN — ничего). Аргументы сертификата не печатаются:
rem в них может быть пароль. Уже подписанный файл не подписывается второй раз.
:sign
if not defined SEMSEARCH_SIGN exit /b 0
set "SS_FILE=%~f1"
powershell -NoProfile -Command "if ((Get-AuthenticodeSignature -LiteralPath $env:SS_FILE).Status -ne 'NotSigned') { exit 1 }"
if errorlevel 1 goto :sign_twice
echo Подписываю %~1
setlocal EnableDelayedExpansion
"!SIGNTOOL!" sign !SEMSEARCH_SIGN! /fd sha256 /tr "!TS!" /td sha256 "!SS_FILE!"
set "RC=!errorlevel!"
endlocal & set "RC=%RC%"
if not "%RC%"=="0" goto :sign_failed
if "%SEMSEARCH_SIGN_TEST%"=="1" goto :sign_verify_test
"%SIGNTOOL%" verify /pa "%SS_FILE%"
if errorlevel 1 goto :sign_bad
exit /b 0
:sign_verify_test
powershell -NoProfile -Command "$s = Get-AuthenticodeSignature -LiteralPath $env:SS_FILE; Write-Host ('  ' + $s.Status + ': ' + $s.SignerCertificate.Subject + '; ' + $s.StatusMessage); if ($s.SignerCertificate -and $s.TimeStamperCertificate -and ('Valid','UnknownError','NotTrusted' -contains [string]$s.Status)) { exit 0 }; exit 1"
if errorlevel 1 goto :sign_bad
exit /b 0
:sign_twice
echo %~1 уже подписан: второй раз не подписываю — пересоберите файл.
exit /b 1
:sign_failed
echo Не удалось подписать %~1 — причина в сообщении signtool выше. Проверьте SEMSEARCH_SIGN и доступ к %TS%.
exit /b 1
:sign_bad
echo Подпись %~1 не прошла проверку.
exit /b 1

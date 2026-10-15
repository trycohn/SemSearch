# Установщик SemSearch (`installer/`)

Один файл `dist\SemSearch-Setup-<версия>.exe`. Ставит программу **для текущего пользователя, без прав
администратора**; Windows спрашивает разрешение только для Visual C++ (если его нет) и для быстрого поиска по имени
(задача движка). Интерфейс — 1:1 по макету «Установщик» (`design/Installer.dc.html`, анимации —
`design/Inst-Motion.dc.html`). Этот файл — общий контракт для частей установщика; меняется вместе с кодом.

## Части

| Часть | Где | Что |
|---|---|---|
| Ядро | `installer/` (Rust, Tauri v2, статический CRT) | Проверки, полезная нагрузка, скачивания, pip, модель, реестр, ярлыки, задача движка, обновление, удаление, тестовый режим, `--headless`. |
| Интерфейс | `installer/ui/` (HTML/CSS/JS без сборщика) | Все экраны макета, RU/EN, светлая/тёмная тема как в Windows. Говорит с ядром только командами и событиями ниже. |
| Модель | `tools/fetch_model.py` | Качает EmbeddingGemma 2 в `D\models` (через `core`), печатает прогресс JSON-строками. |
| Сборка | `build-installer.cmd`, `installer/payload.txt` | Собирает engine и shell, упаковывает файлы программы и Python, дописывает их к exe установщика. |

## Что получается на диске

`<root>` — папка программы, по умолчанию `%LOCALAPPDATA%\Programs\SemSearch` (без прав администратора; папку под
`%ProgramFiles%` не предлагаем — туда не записать).

```
<root>\SemSearch.exe, server.py, app.py, core.py, … (всё по installer/payload.txt)
<root>\ui\, bin\semsearch-engine.exe, engine\install-task.*, engine\uninstall-task.*, tools\fetch_model.py
<root>\requirements.txt, requirements-torch.txt
<root>\runtime\          CPython 3.11 (python-build-standalone, install_only) — из полезной нагрузки
<root>\.venv\            venv из runtime: shell запускает <root>\.venv\Scripts\pythonw.exe <root>\server.py
<root>\uninstall.exe     копия установщика (вызывается с --uninstall)
<root>\install.json      что сделал установщик: version, data_dir, data_dir_env, gpu, mft, autostart, desktop
<root>\.setup-cache\     колесо torch, vc_redist и пр. во время установки; в конце удаляется
```

`D` — папка данных (`docs/API.md` §1), по умолчанию `%LOCALAPPDATA%\SemSearch`, модель — `D\models`. Выбрал
пользователь другую — `HKCU\Environment\SEMSEARCH_DATA_DIR` = путь + рассылка `WM_SETTINGCHANGE` (`data_dir_env: true`
в install.json). Всем дочерним процессам установщика `SEMSEARCH_DATA_DIR` передаётся явно.

Реестр (всё в HKCU):
- `Software\Microsoft\Windows\CurrentVersion\Uninstall\SemSearch`: `DisplayName`=SemSearch, `DisplayVersion`,
  `Publisher`=SemSearch, `DisplayIcon`=`<root>\SemSearch.exe`, `InstallLocation`=`<root>`,
  `UninstallString`=`"<root>\uninstall.exe" --uninstall`, `QuietUninstallString`=`… --uninstall --headless`,
  `EstimatedSize` (КБ, DWORD), `NoModify`=1, `NoRepair`=1.
- Автозапуск — ровно как делает shell (`shell/src/util.rs::set_run_entry`): `Run\SemSearch` =
  `"<root>\SemSearch.exe" --hidden` и `Explorer\StartupApproved\Run\SemSearch` = включено (12 байт: 02 00 …).
- Ярлыки: `%APPDATA%\Microsoft\Windows\Start Menu\Programs\SemSearch.lnk`; на рабочем столе — по выбору.

## Шаги (id — часть контракта, подписи — в интерфейсе)

**Установка** (`mode: "install"`): `check` → [`vcredist`] → `copy` → `torch` → `libs` → `model` → [`mft`] → `links`
- `check` — Windows 10 1709+ (сборка ≥ 16299), 64-бит, место (см. ниже), запись в `<root>` и `D`. Непустая папка,
  не похожая на SemSearch (в `<root>` нет `install.json`/`server.py`/`runtime`/`.venv`/`.setup-cache`, в `D` нет
  `config.json`/`index.db`/`models`/…), — ошибка `other` «папка не пуста»: удаление сносит обе папки целиком.
  Ещё до шагов `start()` отказывает (ошибкой вызова), если папка — корень диска или известная папка (профиль,
  «Документы», `%LOCALAPPDATA%`, Program Files, …) или `<root>` и `D` вложены одна в другую. Есть ли VC++ x64
  (`HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X64` `Installed`=1, также под `WOW6432Node`) — проверяется
  в `start()`, нет → `vcredist` сразу в плане.
- `vcredist` — скачать `https://aka.ms/vs/17/release/vc_redist.x64.exe` в `.setup-cache`, запустить
  `/install /quiet /norestart` через UAC (ShellExecute `runas`). Коды 0, 1638, 3010 — успех. Отказ в UAC —
  ошибка `uac_denied` (дальше без него нельзя: torch и shell не запустятся).
- `copy` — закрыть запущенный SemSearch из этой `<root>` (как `close`), распаковать файлы программы и `runtime\`
  (`runtime\.payload-sha256` совпал — Python не распаковывается заново), `runtime\python.exe -I -m venv <root>\.venv`
  (если venv нет или он сломан; новый `runtime\` при живом venv — `venv --upgrade`), проверить
  `.venv\Scripts\pythonw.exe`, записать `uninstall.exe` (exe установщика без полезной нагрузки). Прогресс —
  `got`/`total` в байтах (файлы + сжатый архив Python).
- `torch` — версии из `requirements-torch.txt` (`torch==…`, `torchvision==…`). Индекс:
  `https://download.pytorch.org/whl/cu128` (видеокарта) или `…/whl/cpu`. Ядро само находит на странице
  `<index>/torch/` колесо `torch-<ver>+<cu128|cpu>-cp311-cp311-win_amd64.whl`, качает его в `.setup-cache` с докачкой
  (HTTP Range), проверяет sha256 из `#sha256=` ссылки. Прогресс — байты (`got`/`total`). Обрыв — ошибка `network`,
  `retry()` докачивает с того же места.
- `libs` — `<venv python> -m pip install <колесо torch> torchvision==<ver> --index-url <index>`, затем
  `pip install -r requirements.txt`. Флаги: `--disable-pip-version-check --no-input --progress-bar off`.
  Сеть — `network`; повтор продолжает (кэш pip). Колесо torch после успеха удаляется.
- `model` — `<venv python> <root>\tools\fetch_model.py` (env `SEMSEARCH_DATA_DIR`=`D`). Скрипт сначала вызывает
  `core.migrate_model()` (модель из общего кэша Hugging Face переносится, а не качается), потом докачивает чего нет.
  Вывод — JSON-строки: `{"got": байт, "total": байт|null}` (не чаще 2 раз в секунду), в конце `{"done": true}` либо
  `{"error": "текст", "kind": "network"|"space"|"other"}` и код выхода 1.
- `mft` — `powershell -NoProfile -ExecutionPolicy Bypass -File <root>\engine\install-task.ps1 -NoPause` (сам
  поднимает права через UAC). Код 0 — успех; 2 — учётная запись не администратор (`not_admin`); 1 — отказ в UAC или
  другая ошибка (`uac_denied` / `other`). **Ошибка `mft` не прерывает установку**: шаг `failed`, дальше `links`,
  в `setup://done` — `mft_ok: false`; `retry_mft()` пробует ещё раз.
- `links` — ярлыки, автозапуск (если выбран), ключ Uninstall, install.json, `SEMSEARCH_DATA_DIR` (если не по
  умолчанию), `D\config.json` — **только если файла ещё нет** и только то, что пользователь сменил в установщике руками: `"lang"`, если
  язык выбран руками (не «как в Windows»), `"theme"`, если выбрана не «как в Windows». Иначе файл не пишется: программа сама берёт
  язык и тему из Windows (язык — русский, если Windows русская, иначе английский). При обновлении автозапуск не
  трогается (после установки им распоряжаются настройки программы), ярлык на рабочем столе обновляется, только если
  он ещё есть. В конце успешного прогона `.setup-cache` удаляется.

**Обновление** (`mode: "update"`, есть ключ Uninstall и `<InstallLocation>\install.json`): `close` → `replace` →
[`torch`] → `libs` → [`mft`] → `links`. Папки, GPU/CPU, автозапуск, MFT — из install.json (менять не даём).
`torch` — только если в venv другая версия torch, чем в `requirements-torch.txt`. `mft` — только если задача
«SemSearch Engine» существует (обновить копию движка). Настройки, индекс и модель не трогаются.

**Удаление** (`mode: "uninstall"`, запуск `uninstall.exe --uninstall`): `close` → [`engine_del`] → `prog_del` →
[`data_del`]
- `uninstall.exe` копирует себя в `%TEMP%\SemSearch-uninstall-<pid>.exe`, запускает копию с
  `--uninstall --root <root>` (и своими `--headless`/`--wipe`/`--test`/`--lang`) и выходит с кодом 0 — с `--headless`
  итог виден по событиям копии (она пишет в ту же консоль), не по коду выхода; копия в конце удаляет себя
  (`cmd /c ping -n 3 127.0.0.1 >nul & del …`). Копия нужна, только если exe лежит внутри удаляемой `<root>`.
- `close` — завершить все процессы, чей exe лежит в `<root>`: `SemSearch.exe`, `pythonw.exe`/`python.exe` из
  `<root>\.venv` (лаунчер venv) и `<root>\runtime` (настоящий интерпретатор), `semsearch-engine.exe` из `<root>\bin`
  (два прохода: shell мог перезапустить backend). Процессы с exe вне `<root>` (WebView2, движок из Program Files) не
  трогаются; задачу движка — тоже.
- `engine_del` — если задача «SemSearch Engine» есть: `engine\uninstall-task.ps1 -NoPause` (UAC). Отказ — шаг
  `failed`, удаление продолжается (в итоге — предупреждение).
- `prog_del` — автозапуск, ярлыки, ключ Uninstall, `SEMSEARCH_DATA_DIR` (если ставили мы), папка `<root>`
  (с повторами: файлы могут отпускаться не сразу), профиль WebView2 программы `%LOCALAPPDATA%\com.semsearch.desktop`
  (кроме тестового режима: у тестовой копии тот же идентификатор).
- `data_del` — если выбрано: папка `D` целиком (настройки, индекс, превью, модель).

Место: нужно **7 ГБ** с видеокартой и **3,5 ГБ** на процессоре (на диске `<root>`; если `D` на другом диске —
там нужно 2 ГБ). Не хватает — `disk_info().ok = false`, интерфейс показывает экран «Мало места».

Видеокарта: `nvidia-smi --query-gpu=name,compute_cap,driver_version --format=csv,noheader,nounits`, берётся GPU с
наибольшим compute capability. Поддерживается, если compute capability ≥ 7.5 (сборка torch cu128 — sm_75 и новее)
**и** драйвер ≥ 528.33 (минимум Windows для колёс CUDA 12.x; сравнение по частям). Иначе — только процессор,
вариант «На видеокарте» неактивен с причиной: `cap` или `driver` (драйвер старый, `nvidia-smi` нет или не
отвечает при видеокарте NVIDIA в системе). Без видеокарты NVIDIA — `gpu: null`.

WebView2 (нужен и установщику, и программе): до создания окна ядро проверяет
`HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` (`pv` не пуст и не
`0.0.0.0`; также HKLM без WOW6432Node и HKCU). Нет — системный MessageBox (RU/EN): «Нужен компонент Microsoft Edge
WebView2. Установить?» → `https://go.microsoft.com/fwlink/p/?LinkId=2124703` → `/silent /install` → продолжить.

Один экземпляр установщика (именованный мьютекс `SemSearchSetup`). Журнал — `%TEMP%\SemSearch-setup.log`
(каждая команда, код выхода, хвост вывода pip; `open_log()` открывает его).

## Команды (Tauri `invoke`) и события

Окно 720×700, без рамки, не растягивается; заголовок рисует интерфейс (`data-tauri-drag-region`), кнопки
«Свернуть»/«Закрыть» — через API окна. `withGlobalTauri: true` → `window.__TAURI__.core.invoke`,
`window.__TAURI__.event.listen`.

Аргументы команд — ключи snake_case, как ниже (лишние ключи, например camelCase-копии, игнорируются); у `start`
все необязательны (обновление и удаление берут своё из install.json, удаление — только `wipe`).

- `get_state()` → `{mode: "install"|"update"|"uninstall", version, lang: "ru"|"en", test: bool,
  installed: {version, root, data_dir, gpu, mft, autostart, desktop} | null,
  defaults: {root, data_dir}, gpu: {name, cap, driver, supported, reason?} | null, windows: {build, ok}}`
  (`lang` — язык интерфейса Windows: русский → `ru`, иначе `en`; `gpu` — см. «Видеокарта» выше: `reason` —
  `"cap"` | `"driver"`; без ответа `nvidia-smi` `cap` и `driver` — `""`, `name` — из списка устройств Windows).
- `disk_info({root, data_dir, gpu})` → `{drive, free_bytes, need_bytes, ok, data_drive?, data_free_bytes?}`
- `pick_folder({initial})` → путь | null (системный диалог выбора папки).
- `start({root, data_dir, gpu, autostart, mft, desktop, lang, theme, wipe})` (`lang`: `system`|`ru`|`en`, `theme`: `system`|`light`|`dark` — выпадающие списки
  в подвале приветствия, как в настройках программы; по умолчанию оба «как в Windows») — запускает шаги текущего `mode`. Можно
  вызвать снова после ошибки или `cancel()` (например, с другой папкой после «Мало места»); если прогон ещё
  останавливается после `cancel()`, `start` его дожидается.
- `retry()` — повторить упавший шаг и продолжить. `cancel()` — остановить (убить pip/скрипт, докачку оставить):
  шаг заканчивается `failed` + `setup://error` с `kind: "cancelled"`; если ничего не идёт — ничего не происходит.
  Закрытие окна во время шага делает то же перед выходом.
- `retry_mft()` — снова `mft` (после установки), результат — событием `setup://step` для `mft`.
- `finish({launch})` — `launch`: запустить `<root>\SemSearch.exe` (отдельным процессом, с `SEMSEARCH_DATA_DIR`),
  затем закрыть установщик. `open_log()`, `open_path({path})` (только существующий путь).

События:
- `setup://plan` `{mode, steps: [id, …]}` — в начале `start`.
- `setup://step` `{id, state: "active"|"done"|"failed"|"skipped", got?, total?, note?, kind?, message?}`
  - `active` с `got`/`total` — прогресс в байтах (`copy`/`replace`, `vcredist`, `torch`, `model`), не чаще 4 раз в
    секунду; `total` бывает `null`.
  - `active` с `note` — код, не текст: `"uac"` (ждём ответа на запрос Windows: `vcredist`, `mft`, `engine_del`),
    `"verify"` (проверка sha256 скачанного), `"python"` (распаковка Python), `"venv"` (создание venv).
  - `failed` — всегда с `kind` и `message` (как в `setup://error`); у шагов со скачиванием — ещё `got`/`total`,
    где остановились (`torch`: и при повторе, если сеть отказала до продолжения докачки).
- `setup://error` `{id, kind: "network"|"space"|"uac_denied"|"not_admin"|"cancelled"|"other", message, got?, total?}`
  — после `failed` у фатального шага; установщик ждёт `retry()` / `cancel()` / `start()`. У `mft` и `engine_del`
  (не фатальны) — только `failed` у шага, без `setup://error`.
- `setup://done` `{mode, ok: true, mft_ok, root, data_dir, hotkey}` (`hotkey` — для экрана «Готово»: из `D\config.json`, иначе
  как выберет программа: Ctrl+Alt+S, а если AltGr+S набирает букву в одной из раскладок — Ctrl+Alt+Space) — только после успешного прогона (ошибка приходит
  `setup://error`, `done` с `ok: false` ядро не шлёт). `mft_ok: false` — `mft` (или при удалении `engine_del`) был в
  плане и не удался; иначе `true`.

Как интерфейс показывает ошибки: `network` у `torch`/`libs`/`model`/`vcredist` → экран «Нет сети» (подпись шага,
«Повторить»); `space` → «Мало места»; `mft` с `not_admin`/`uac_denied` → «Готово» с предупреждением и кнопкой
«Подключить» (`retry_mft`); остальное — экран ошибки с текстом, «Открыть журнал» и «Повторить».

## Тестовый режим и `--headless`

`SEMSEARCH_SETUP_TEST=1` — для проверки на машине разработчика, где уже стоит SemSearch:
- имя продукта `SemSearch-Test`: ключ Uninstall, значение автозапуска, ярлыки «SemSearch (test)»;
- `mft`, `engine_del` — всегда `skipped` (задача «SemSearch Engine» общая, её трогать нельзя);
- по умолчанию `<root>` = `%TEMP%\SemSearch-Test\app`, `D` = `%TEMP%\SemSearch-Test\data`;
- дочерним процессам — `HF_HOME=<root>\.setup-cache\hf-home`, чтобы `core.migrate_model()` не утащил настоящую
  модель из `%USERPROFILE%\.cache\huggingface` (и без `HF_HUB_CACHE`/`HUGGINGFACE_HUB_CACHE`/`HF_XET_CACHE`);
- `HKCU\Environment\SEMSEARCH_DATA_DIR` не пишется и не удаляется (`data_dir_env: false`);
- `finish({launch: true})` SemSearch не запускает: автозапуск shell'а переписал бы настоящий `Run\SemSearch` на
  тестовую копию;
- `start()` отказывает, если `<root>` или `D` совпадает/пересекается с папками настоящей установки
  (`%LOCALAPPDATA%\Programs\SemSearch`, `%LOCALAPPDATA%\SemSearch`, `InstallLocation` ключа `SemSearch`,
  `%USERPROFILE%\.cache\huggingface`);
- то же включает ключ `--test`; install.json получает `"test": true`, а `UninstallString` — `--test`: такая
  установка удаляется и обновляется в тестовом режиме, что бы ни стояло в окружении.

`SemSearch-Setup.exe --headless [--root P] [--data P] [--cpu|--gpu] [--no-mft] [--no-autostart] [--desktop]
[--lang ru|en] [--test]` — те же шаги без окна, события — JSON-строками в stdout (консоль родителя), код выхода 0/1.
`--headless --uninstall --root P [--wipe]` — удаление. Используется тестами и для тихой установки. Без `--cpu`/`--gpu`
— видеокарта, если поддерживается. Есть установка этого продукта (ключ Uninstall + install.json) — это обновление.
Строка события — один JSON-объект: `{"event": "setup://step", "id": "torch", "state": "active", "got": …}` (поля
события рядом с `event`). exe — оконная программа: из cmd — `start /wait`, иначе cmd не ждёт; PowerShell и
перенаправленный вывод (`| …`, `> файл`) ждут сами. Ошибка до шагов (`start()` отказал) —
`{"event": "setup://error", "id": "start", "kind": "other", "message": …}`.

## Сборка

`build-installer.cmd`: `build.cmd` (engine + shell) → `cargo build --release` в `installer\` (собирает и
`SemSearch-Setup.exe`, и упаковщик `pack.exe`) → копия голого `SemSearch-Setup.exe` в `installer\target\uninstall.exe`;
подпись `SemSearch.exe`, `bin\semsearch-engine.exe` и этой копии → `pack python`: Python (python-build-standalone
CPython 3.11, x86_64 windows-msvc install_only; закреплённые URL и sha256 — в `installer/python.txt`) в
`installer\.cache\` (с докачкой, sha256) → `pack zip`: файлы по `installer/payload.txt` + Python + `uninstall.exe` →
`pack append`: полезная нагрузка дописывается к **неподписанному** голому exe
(`[exe][zip][u64 длина, little-endian][8 байт "SSPAYLD1"]`) → подпись готового `dist\SemSearch-Setup-<версия>.exe` →
`pack info` проверяет, что нагрузка находится. Каждый файл подписывается один раз (уже подписанный — ошибка сборки).
Версия — из `shell/tauri.conf.json` (её же `build.rs` вшивает в установщик). Для разработки без упаковки:
`SEMSEARCH_PAYLOAD_DIR=<корень репозитория>` — ядро берёт файлы прямо оттуда (по тому же payload.txt), Python — из
`installer\.cache\` (один раз: `installer\target\release\pack.exe python .`).

В zip: `app/<путь от корня репозитория>` — файлы программы, `python.tar.gz` (без сжатия), `uninstall.exe` (голый exe
установщика, со своей подписью; шаг `copy` кладёт его в `<root>`, а без него — как раньше, exe без нагрузки),
`payload.json` (`{"python": "<имя архива>", "python_sha256": "…", "files": N}`).

Подпись Authenticode (необязательна; без `SEMSEARCH_SIGN` — «Подпись пропущена…» и сборка без подписи).
signtool кладёт таблицу сертификатов (каталог безопасности PE, №4) в конец файла, после нагрузки, выравнивая её
нулями до 8 байт; хэш подписи покрывает и нагрузку. Поэтому ядро ищет нагрузку не от конца файла, а от начала
таблицы сертификатов, если она есть и доходит до конца, допуская до 7 нулевых байт после `SSPAYLD1`.
- `SEMSEARCH_SIGN` — аргументы signtool, выбирающие сертификат (не печатаются: в них бывает пароль):
  - файл: `/f C:\keys\semsearch.pfx /p <пароль>`;
  - хранилище Windows или аппаратный токен: `/sha1 <отпечаток сертификата>`;
  - Azure Trusted Signing: `/dlib <…\Azure.CodeSigning.Dlib.dll> /dmdf <…\metadata.json>`.
- `SEMSEARCH_SIGNTOOL` — путь к signtool.exe; по умолчанию самый новый `Windows Kits\10\bin\<версия>\x64\signtool.exe`.
- `SEMSEARCH_SIGN_TS` — сервер меток времени RFC 3161 (по умолчанию `http://timestamp.digicert.com`); всегда
  добавляются `/fd sha256 /tr <сервер> /td sha256`.
- После каждой подписи — `signtool verify /pa`. `SEMSEARCH_SIGN_TEST=1` (самоподписанный тестовый сертификат):
  недоверенный корень допустим, но подпись и метка времени должны быть, а хэш — целым (`Get-AuthenticodeSignature`).

`installer/payload.txt`: одна маска на строку от корня репозитория (`/` или `\`); `#` — комментарий, пустые строки
пропускаются; `*` и `?` не выходят за папку, `**` — через папки, `[abc]` — класс, регистр не важен; `папка/` = всё
внутри; `!маска` исключает (после всех включений, порядок строк не важен). Маска без совпадений (кроме `!`) —
ошибка сборки. Обход не заходит в `.git`, `.claude`, `.venv`, `target`, `__pycache__`, `node_modules` (на любой
глубине) и в `data\`, `dist\`, `installer\.cache\` в корне.

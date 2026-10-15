/* SemSearch Setup — texts RU/EN. Wording 1:1 with design/Installer.dc.html (T(en)); strings the mockup lacks
   (Visual C++, error screen, unsupported GPU, failures of the engine task) are written in the same voice.
   Sizes: bytes are shown in binary units (1 GB = 1024^3 bytes), as Windows Explorer does. */
(function () {
  'use strict';
  const GiB = 1073741824, MiB = 1048576;

  function make(en) {
    const dec = s => en ? s : s.replace('.', ',');
    const GB = en ? ' GB' : ' ГБ', MB = en ? ' MB' : ' МБ';
    // 7 GB, 3.5 GB, 214 GB, 3.1 GB, 850 MB
    const size = b => {
      const g = (b || 0) / GiB;
      if (g >= 10) return Math.round(g) + GB;
      if (g >= 0.95) return dec(g.toFixed(1).replace(/\.0$/, '')) + GB;
      return Math.max(0, Math.round((b || 0) / MiB)) + MB;
    };
    // «1,3 из 3,0 ГБ» / "1.3 of 3.0 GB"; small totals in MB; unknown total — just what came
    const of = (got, total) => {
      got = got || 0;
      const big = (total || got) >= 0.95 * GiB;
      const f = big ? (x => dec((x / GiB).toFixed(1))) : (x => String(Math.round(x / MiB)));
      const u = big ? GB : MB;
      if (!total) return f(got) + u;
      return f(got) + (en ? ' of ' : ' из ') + f(total) + u;
    };

    if (en) return {
      size, of,
      bar: { install: 'SemSearch Setup', update: 'SemSearch Update', uninstall: 'Uninstall SemSearch' },
      min: 'Minimize', close: 'Close', langAria: 'Setup language', langName: 'English', themeAria: 'Theme', themeNames: { system: 'Same as Windows', light: 'Light', dark: 'Dark' }, sysLang: 'Same as Windows',
      welcomeSub: 'Search your computer by meaning and by name. Your files never leave this PC.',
      progFolder: 'Program folder', dataFolder: 'Data folder', dataNote: 'settings, index and model',
      onGpu: 'The model runs on the graphics card', onCpu: 'The model runs on the processor', cpuShort: 'Slower indexing, smaller download',
      mftOn: 'Fast search by name', mftOnNote: 'Windows will ask for administrator permission once', mftOff: 'Search by name walks the disk', mftOffNote: 'No administrator rights; the first index takes longer',
      space: need => 'Takes about ' + need, free: (f, d) => f + ' free on drive ' + d, freeShort: (f, d) => f + ' free on ' + d,
      lowLabel: d => 'Not enough space on drive ' + d, lowNote: (need, f) => 'Needs about ' + need + ', ' + f + ' free',
      lowFoot: (d, need, f) => 'Not enough space on ' + d + ' — needs about ' + need + ', ' + f + ' free', otherFolder: 'Another folder',
      settings: 'Options', install: 'Install',
      optTitle: 'Setup options', where: 'Where to install', change: 'Change', computeOn: 'Run the model on',
      gpu: 'Graphics card', gpuNote: name => name + ' · fast · about 4.5 GB to download', cpu: 'Processor', cpuNote: 'Slower indexing · about 1.7 GB to download', startSec: 'Startup',
      gpuNone: 'No NVIDIA graphics card found',
      gpuOld: name => name + ' · too old: needs a GTX 16, RTX 20 or newer',
      gpuDriver: name => name + ' · needs a newer NVIDIA driver',
      auto: 'Start with Windows', autoNote: 'Runs in the tray; the shortcut opens the window',
      mft: 'Fast search by name', mftNote: 'Name index in seconds. Windows will ask for administrator rights once.',
      desk: 'Desktop shortcut', deskNote: 'The Start menu shortcut is always created',
      back: 'Back', progTitle: 'Installing SemSearch', canMin: 'You can minimize this window — setup keeps going', cancel: 'Cancel', readyNow: 'Done',
      netTitle: { torch: 'Couldn’t download PyTorch', model: 'Couldn’t download the model', libs: 'Couldn’t install the libraries', vcredist: 'Couldn’t download Visual C++' },
      netSub: s => 'The connection dropped at ' + s + '. The downloaded part is kept — setup will resume from there.',
      netSubPlain: 'The connection dropped. The downloaded part is kept — setup will resume from there.',
      netSubRetry: 'The connection dropped. Check the internet connection and retry.',
      errTitle: { install: 'Couldn’t install SemSearch', update: 'Couldn’t update SemSearch', uninstall: 'Couldn’t uninstall SemSearch' },
      errSubLog: 'The details are in the log.',
      vcTitle: 'Visual C++ isn’t installed', vcUacSub: 'Windows didn’t grant permission. SemSearch can’t run without Visual C++ — retry and allow the install.',
      uacSub: 'Windows didn’t grant permission. Retry and allow it.',
      errRight: { network: 'no connection', uac_denied: 'not allowed', not_admin: 'no administrator rights', space: 'out of space', other: 'failed' },
      retry: 'Retry', log: 'Open the log',
      doneTitle: 'SemSearch is installed', doneSub: 'Open the search window from any app — Ctrl+Alt+S',
      firstRun: 'The model is downloaded', firstRunNote: 'EmbeddingGemma 2 is in place — the first start downloads nothing',
      byName: 'Search by name works right away', byNameNote: 'Search by meaning grows as indexing runs in the background',
      tray: 'SemSearch lives in the tray', trayAuto: 'Starts with Windows', trayNoAuto: 'Start it from the Start menu',
      noAdm: 'Fast search by name isn’t set up', noAdmNote: 'Windows didn’t grant permission. Names are still found, but the first index takes longer.', connect: 'Set up',
      engNotUp: 'The name search engine isn’t updated', engNotUpNote: 'Windows didn’t grant permission. Search by name keeps working on the previous version.',
      engNotDel: 'The name search engine isn’t removed', engNotDelNote: 'Windows didn’t grant permission. The “SemSearch Engine” task is still in Task Scheduler.',
      launch: 'Start SemSearch', finish: 'Done',
      updTitle: 'Update SemSearch', updSub: (a, b) => a === b ? 'Version ' + a + ' is already installed — its program files will be replaced.' : 'Version ' + a + ' is installed, ' + b + ' is ready.',
      keep: 'Settings, index and model stay', keepNote: 'Nothing to download or index again', closes: 'SemSearch closes during the update', closesNote: 'It opens again by itself when everything is ready', eng: 'The name search engine gets updated', engNote: 'Windows will ask for administrator permission',
      notNow: 'Not now', update: 'Update', updating: 'Updating SemSearch', updNote: 'SemSearch will open again by itself', updated: 'SemSearch is updated', updatedSub: v => 'Version ' + v + '. Settings and index are in place.',
      unTitle: 'Uninstall SemSearch?', unSub: 'The program, its shortcuts and the name search engine will be removed.', unProg: 'Program and name search engine', unData: 'Also remove settings, index and model', unDataNote: 'Keep them if you’ll install again — no need to download the model',
      remove: 'Uninstall', removing: 'Uninstalling SemSearch', removed: 'SemSearch is uninstalled', removedKeep: 'Settings, index and model are still in the data folder.', removedAll: 'The program and all its data are removed.', openFolder: 'Open folder', closeBtn: 'Close',
      sCheck: 'Checking this PC', sCheckOk: 'all good', sVc: 'Installing Visual C++', sVcWait: 'Windows will ask', sCopy: 'Copying program files', sTorchGpu: 'Downloading PyTorch with CUDA', sTorchCpu: 'Downloading PyTorch for the processor', sLibs: 'Installing libraries', sModel: 'Downloading the EmbeddingGemma 2 model', sMft: 'Setting up fast search by name', sWaitWin: 'waiting for Windows', sLinks: 'Creating shortcuts', sLinksAuto: 'Creating shortcuts and autostart', sClose: 'Closing SemSearch', sReplace: 'Replacing program files', sLibsUp: 'Updating libraries', sEngUp: 'Updating the name search engine', sEngDel: 'Removing the name search engine', sProgDel: 'Removing the program and shortcuts', sDataDel: 'Removing settings, index and model', sSkipped: 'skipped'
    };

    return {
      size, of,
      bar: { install: 'Установка SemSearch', update: 'Обновление SemSearch', uninstall: 'Удаление SemSearch' },
      min: 'Свернуть', close: 'Закрыть', langAria: 'Язык установки', langName: 'Русский', themeAria: 'Тема', themeNames: { system: 'Как в Windows', light: 'Светлая', dark: 'Тёмная' }, sysLang: 'Как в Windows',
      welcomeSub: 'Поиск по компьютеру по смыслу и по имени. Ваши файлы не покидают компьютер.',
      progFolder: 'Папка программы', dataFolder: 'Папка данных', dataNote: 'настройки, индекс и модель',
      onGpu: 'Модель считает на видеокарте', onCpu: 'Модель считает на процессоре', cpuShort: 'Индексация медленнее, скачивать меньше',
      mftOn: 'Быстрый поиск по имени', mftOnNote: 'Windows один раз спросит разрешение администратора', mftOff: 'Поиск по имени — обходом диска', mftOffNote: 'Без прав администратора: первый индекс строится дольше',
      space: need => 'Займёт около ' + need, free: (f, d) => 'Свободно ' + f + ' на диске ' + d, freeShort: (f, d) => 'свободно ' + f + ' на ' + d,
      lowLabel: d => 'Не хватает места на диске ' + d, lowNote: (need, f) => 'Нужно около ' + need + ', свободно ' + f,
      lowFoot: (d, need, f) => 'Не хватает места на ' + d + ' — нужно около ' + need + ', свободно ' + f, otherFolder: 'Другая папка',
      settings: 'Параметры', install: 'Установить',
      optTitle: 'Параметры установки', where: 'Куда установить', change: 'Изменить', computeOn: 'На чём считать модель',
      gpu: 'На видеокарте', gpuNote: name => name + ' · быстро · скачать около 4,5 ГБ', cpu: 'На процессоре', cpuNote: 'Индексация медленнее · скачать около 1,7 ГБ', startSec: 'Запуск',
      gpuNone: 'Видеокарта NVIDIA не найдена',
      gpuOld: name => name + ' · слишком старая: нужна GTX 16, RTX 20 или новее',
      gpuDriver: name => name + ' · нужен драйвер NVIDIA новее',
      auto: 'Запускать вместе с Windows', autoNote: 'Работает в трее, окно открывается по горячей клавише',
      mft: 'Быстрый поиск по имени', mftNote: 'Индекс имён за секунды. Windows один раз спросит права администратора.',
      desk: 'Ярлык на рабочем столе', deskNote: 'В меню «Пуск» ярлык появится в любом случае',
      back: 'Назад', progTitle: 'Устанавливаю SemSearch', canMin: 'Окно можно свернуть — установка продолжится', cancel: 'Отмена', readyNow: 'Готово',
      netTitle: { torch: 'Не удалось скачать PyTorch', model: 'Не удалось скачать модель', libs: 'Не удалось поставить библиотеки', vcredist: 'Не удалось скачать Visual C++' },
      netSub: s => 'Связь прервалась на ' + s + '. Скачанное сохранено — продолжу с того же места.',
      netSubPlain: 'Связь прервалась. Скачанное сохранено — продолжу с того же места.',
      netSubRetry: 'Связь прервалась. Проверьте интернет и повторите.',
      errTitle: { install: 'Не удалось установить SemSearch', update: 'Не удалось обновить SemSearch', uninstall: 'Не удалось удалить SemSearch' },
      errSubLog: 'Подробности — в журнале.',
      vcTitle: 'Visual C++ не установлен', vcUacSub: 'Windows не дала разрешения. Без Visual C++ SemSearch не запустится — повторите и разрешите установку.',
      uacSub: 'Windows не дала разрешения. Повторите и разрешите.',
      errRight: { network: 'нет связи', uac_denied: 'нет разрешения', not_admin: 'нет прав администратора', space: 'мало места', other: 'ошибка' },
      retry: 'Повторить', log: 'Открыть журнал',
      doneTitle: 'SemSearch установлен', doneSub: 'Окно поиска открывается из любой программы — Ctrl+Alt+S',
      firstRun: 'Модель уже скачана', firstRunNote: 'EmbeddingGemma 2 на месте — первый запуск ничего не качает',
      byName: 'Поиск по имени работает сразу', byNameNote: 'По смыслу — по мере индексации, она идёт в фоне',
      tray: 'SemSearch живёт в трее', trayAuto: 'Запускается вместе с Windows', trayNoAuto: 'Запускайте из меню «Пуск»',
      noAdm: 'Быстрый поиск по имени не подключён', noAdmNote: 'Windows не дала разрешения. Имена найдутся и так, но первый индекс строится дольше.', connect: 'Подключить',
      engNotUp: 'Движок поиска по имени не обновлён', engNotUpNote: 'Windows не дала разрешения. Поиск по имени работает на прежней версии.',
      engNotDel: 'Движок поиска по имени не удалён', engNotDelNote: 'Windows не дала разрешения. Задача «SemSearch Engine» осталась в Планировщике заданий.',
      launch: 'Запустить SemSearch', finish: 'Готово',
      updTitle: 'Обновить SemSearch', updSub: (a, b) => a === b ? 'Версия ' + a + ' уже установлена — файлы программы будут заменены.' : 'Установлена версия ' + a + ', готова ' + b + '.',
      keep: 'Настройки, индекс и модель сохранятся', keepNote: 'Ничего не придётся скачивать и индексировать заново', closes: 'SemSearch закроется на время обновления', closesNote: 'Когда всё будет готово, откроется сам', eng: 'Движок поиска по имени обновится', engNote: 'Windows спросит разрешение администратора',
      notNow: 'Не сейчас', update: 'Обновить', updating: 'Обновляю SemSearch', updNote: 'SemSearch откроется сам, когда всё будет готово', updated: 'SemSearch обновлён', updatedSub: v => 'Версия ' + v + '. Настройки и индекс на месте.',
      unTitle: 'Удалить SemSearch?', unSub: 'Программа, ярлыки и движок поиска по имени будут удалены.', unProg: 'Программа и движок поиска по имени', unData: 'Удалить также настройки, индекс и модель', unDataNote: 'Оставьте, если будете ставить снова: модель не придётся скачивать',
      remove: 'Удалить', removing: 'Удаляю SemSearch', removed: 'SemSearch удалён', removedKeep: 'Настройки, индекс и модель остались в папке данных.', removedAll: 'Программа и все её данные удалены.', openFolder: 'Открыть папку', closeBtn: 'Закрыть',
      sCheck: 'Проверяю компьютер', sCheckOk: 'всё подходит', sVc: 'Ставлю Visual C++', sVcWait: 'спросит Windows', sCopy: 'Копирую файлы программы', sTorchGpu: 'Скачиваю PyTorch с CUDA', sTorchCpu: 'Скачиваю PyTorch для процессора', sLibs: 'Ставлю библиотеки', sModel: 'Скачиваю модель EmbeddingGemma 2', sMft: 'Подключаю быстрый поиск по имени', sWaitWin: 'ждёт разрешения Windows', sLinks: 'Создаю ярлыки', sLinksAuto: 'Создаю ярлыки и автозапуск', sClose: 'Закрываю SemSearch', sReplace: 'Заменяю файлы программы', sLibsUp: 'Обновляю библиотеки', sEngUp: 'Обновляю движок поиска по имени', sEngDel: 'Удаляю движок поиска по имени', sProgDel: 'Удаляю программу и ярлыки', sDataDel: 'Удаляю настройки, индекс и модель', sSkipped: 'пропущен'
    };
  }

  window.SSI = { ru: make(false), en: make(true), GiB, MiB };
})();

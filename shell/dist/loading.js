// Bundled placeholder page: shown by the shell until the backend answers (window "main"/"settings" -> index.html,
// window "tray" -> tray.html). The shell pushes an extra status line via window.__semsearchStatus(text) and sets
// window.__SEMSEARCH_BOOT = {lang, sys, theme, accent} before the page runs (language and theme from config.json).
(function () {
  var T = window.__TAURI__;
  var boot = window.__SEMSEARCH_BOOT || {};
  var TEXT = {
    ru: { starting: 'Запускаю SemSearch…', index: 'Открываю индекс', model: 'Загружаю модель', vectors: 'Загружаю векторы в память',
      watch: 'Включаю слежение за файлами', progress: 'Запуск', min: 'Свернуть', max: 'Развернуть', hide: 'Скрыть в трей',
      trayStarting: 'Запускаю…', open: 'Открыть поиск', quit: 'Выйти из SemSearch' },
    en: { starting: 'Starting SemSearch…', index: 'Opening the index', model: 'Loading the model', vectors: 'Loading vectors into memory',
      watch: 'Watching files for changes', progress: 'Startup', min: 'Minimize', max: 'Maximize', hide: 'Hide to tray',
      trayStarting: 'Starting…', open: 'Open search', quit: 'Quit SemSearch' }
  };
  var lang = boot.lang === 'en' || boot.lang === 'ru' ? boot.lang : boot.sys === 'en' || boot.sys === 'ru' ? boot.sys
    : /^ru\b/i.test(navigator.language || '') ? 'ru' : 'en';
  var t = TEXT[lang];
  document.documentElement.setAttribute('lang', lang);
  function translate() {
    Array.prototype.forEach.call(document.querySelectorAll('[data-t]'), function (el) {
      var k = el.getAttribute('data-t');
      if (!t[k]) return;
      if (el.tagName === 'BUTTON') el.setAttribute('aria-label', t[k]);
      else el.textContent = t[k];
    });
    Array.prototype.forEach.call(document.querySelectorAll('[data-t-label]'), function (el) {
      el.setAttribute('aria-label', t[el.getAttribute('data-t-label')] || '');
    });
  }

  /* тема: как у страниц интерфейса (ui/common.js applyTheme), акцент в тёмной теме — светлее на 45% */
  var root = document.documentElement;
  var dark = boot.theme === 'dark' || (boot.theme !== 'light' && window.matchMedia && matchMedia('(prefers-color-scheme: dark)').matches);
  if (boot.theme === 'dark' || boot.theme === 'light') root.classList.add(boot.theme);
  if (/^#[0-9a-f]{6}$/i.test(boot.accent || '')) {
    var n = parseInt(boot.accent.slice(1), 16);
    var f = function (c) { return Math.round(c + (255 - c) * 0.45).toString(16).padStart(2, '0'); };
    root.style.setProperty('--accent', dark ? '#' + f((n >> 16) & 255) + f((n >> 8) & 255) + f(n & 255) : boot.accent);
  }

  var def = null;
  window.__semsearchStatus = function (text) {
    var el = document.getElementById('status');
    if (!el || typeof text !== 'string') return;
    if (def === null) def = el.textContent;
    el.textContent = text || def;
  };
  document.addEventListener('DOMContentLoaded', translate);
  if (document.readyState !== 'loading') translate();
  /* кнопки окна — как у страницы, которая придёт на смену: у главного окна есть «Развернуть», у настроек нет */
  try {
    if (T && T.window && T.window.getCurrentWindow().label !== 'main') {
      var mx = document.getElementById('capMax');
      if (mx) mx.parentNode.removeChild(mx);
    }
  } catch (e) { /* не в оболочке */ }
  document.addEventListener('click', function (e) {
    var b = e.target.closest ? e.target.closest('[data-cmd]') : null;
    if (b && T && T.core) T.core.invoke(b.getAttribute('data-cmd')).catch(function () {});
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && document.body.classList.contains('tray') && T && T.core) T.core.invoke('tray_hide').catch(function () {});
  });
  document.addEventListener('contextmenu', function (e) { e.preventDefault(); });
})();

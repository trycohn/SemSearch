// Bundled placeholder page: shown by the shell until the backend answers (window "main"/"settings" -> index.html,
// window "tray" -> tray.html). The shell pushes an extra status line via window.__semsearchStatus(text).
(function () {
  var T = window.__TAURI__;
  var def = null;
  window.__semsearchStatus = function (text) {
    var el = document.getElementById('status');
    if (!el || typeof text !== 'string') return;
    if (def === null) def = el.textContent;
    el.textContent = text || def;
  };
  document.addEventListener('click', function (e) {
    var b = e.target.closest ? e.target.closest('[data-cmd]') : null;
    if (b && T && T.core) T.core.invoke(b.getAttribute('data-cmd')).catch(function () {});
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && document.body.classList.contains('tray') && T && T.core) T.core.invoke('tray_hide').catch(function () {});
  });
  document.addEventListener('contextmenu', function (e) { e.preventDefault(); });
})();

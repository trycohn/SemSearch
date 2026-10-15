/* SemSearch — попап трея. */
(function () {
  'use strict';
  const $ = (id) => document.getElementById(id);
  let st = null, timer = 0;

  async function refresh() {
    clearTimeout(timer);
    try { st = await SS.api('/api/status'); } catch (e) { st = null; }
    const info = R.status(st);
    $('st').textContent = info.kind === 'done' ? SS.t('tray.done') : info.text;
    $('st').title = info.text;
    const bar = $('bar');
    bar.hidden = info.pct === undefined;
    if (info.pct !== undefined) {
      $('barFill').style.transform = 'scaleX(' + (info.pct / 100).toFixed(4) + ')';
      bar.setAttribute('aria-label', info.label);
      bar.setAttribute('aria-valuemax', String(info.max));
      bar.setAttribute('aria-valuenow', String(info.now));
    }
    const manual = !!(st && st.paused && st.paused.reason === 'manual');
    $('pauseIco').setAttribute('d', manual ? SS.ic.play : SS.ic.pause);
    $('pauseLbl').textContent = SS.t(manual ? 'tray.resume' : 'tray.pause');
    $('mPause').disabled = !st;
    if (!document.hidden) timer = setTimeout(refresh, 2000);
  }
  async function settings() {
    try {
      const s = await SS.api('/api/settings');
      SS.applyTheme(s.theme, s.accent);
      SS.setLang(s.lang, s.lang_system);
      $('hk').textContent = SS.caps(s.hotkey || 'ctrl+alt+s').join('+');
    } catch (e) { /* backend ещё не готов */ }
  }
  const menu = $('menu');
  const play = () => {
    menu.classList.remove('in', 'out');
    void menu.offsetWidth;
    menu.classList.add('in');
  };
  let hiding = false;
  const hide = () => {
    if (!SS.tauri || hiding) return Promise.resolve();
    if (SS.reduced()) return SS.invoke('tray_hide').catch(() => {});
    hiding = true;
    menu.classList.remove('in');
    menu.classList.add('out');
    return new Promise((res) => SS.after(menu, 100, () => {
      hiding = false;
      SS.invoke('tray_hide').catch(() => {}).then(res);
    }));
  };

  $('mOpen').addEventListener('click', async () => {
    if (SS.tauri) { await SS.invoke('show_main').catch(() => {}); hide(); } else window.open('/', '_blank', 'noopener');
  });
  $('mPause').addEventListener('click', async () => {
    const manual = !!(st && st.paused && st.paused.reason === 'manual');
    try { await SS.api('/api/pause', { body: { paused: !manual } }); } catch (e) { /* статус покажет */ }
    await refresh();
    hide();
  });
  $('mRescan').addEventListener('click', async () => {
    try { await SS.api('/api/rescan', { body: {} }); } catch (e) { /* статус покажет */ }
    hide();
  });
  $('mSettings').addEventListener('click', async () => {
    if (SS.tauri) { await SS.invoke('show_settings').catch(() => {}); hide(); } else window.open('/settings', '_blank', 'noopener');
  });
  $('mQuit').addEventListener('click', () => {
    if (SS.tauri) SS.invoke('quit_app').catch(() => {});
    else SS.api('/api/shutdown', { body: {} }).catch(() => {});
  });

  document.addEventListener('keydown', (e) => {
    const items = Array.from(document.querySelectorAll('.mi:not(:disabled)'));
    const i = items.indexOf(document.activeElement);
    if (e.key === 'Escape') { e.preventDefault(); hide(); }
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const n = i < 0 ? (e.key === 'ArrowDown' ? 0 : items.length - 1) : (i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length;
      items[n].focus();
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      items[e.key === 'Home' ? 0 : items.length - 1].focus();
    }
  });
  window.addEventListener('blur', () => { if (SS.tauri) hide(); });
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) { clearTimeout(timer); menu.classList.remove('in'); menu.classList.add('out'); return; }
    refresh();
    settings();
  });
  SS.listen('tray-shown', () => { play(); refresh(); settings(); });
  SS.onLang(() => refresh());

  refresh();
  settings();
})();

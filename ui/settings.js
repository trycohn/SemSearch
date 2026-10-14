/* SemSearch — окно настроек: GET/POST /api/settings + команды оболочки; поведение элементов — design/Controls. */
(function () {
  'use strict';
  const { h, svg, ic } = SS;
  const $ = (id) => document.getElementById(id);
  let S = null;
  let fresh = '';          // только что добавленная папка («считаю файлы…»)
  // правило «чужого кода» (config foreign_code)
  const FOREIGN = { skip: 'Не индексировать', last: 'В последнюю очередь', normal: 'Как обычный код' };

  /* ---------- сохранение: применяется сразу, внизу — «Сохранено» ---------- */
  async function save(patch, okText) {
    try {
      const r = await SS.api('/api/settings', { body: patch });
      Object.keys(patch).forEach((k) => {
        if (k === 'content_roots' || k === 'drives') return;
        S[k] = patch[k];
      });
      SS.toast(r && r.restart ? 'Применится после перезапуска' : okText || 'Сохранено');
      return true;
    } catch (e) {
      SS.toast('Не удалось сохранить: ' + (e.detail || 'SemSearch не отвечает'));
      return false;
    }
  }
  /* Окно настроек создаётся скрытым и показывается, только когда страница отрисована с данными —
     без мелькания пустого окна и страницы-заготовки */
  let readySent = false;
  const pageReady = () => {
    if (readySent || !SS.tauri) return;
    readySent = true;
    SS.invoke('page_ready').catch(() => {});
  };
  async function reload() {
    try {
      S = await SS.api('/api/settings');
    } catch (e) {
      SS.toast('Не удалось загрузить настройки');
      pageReady();
      return;
    }
    SS.applyTheme(S.theme, S.accent);
    render();
    pageReady();
  }

  /* ---------- отрисовка ---------- */
  const xIcon = (size) => svg(ic.x, { s: size || 14, sw: size === 12 ? 2 : 1.8 });
  const chkIcon = () => svg(ic.check, { s: 12, sw: 2.6, join: true, cls: 'pop' });
  const filesNote = (r) => (r.path === fresh && !r.files ? 'считаю файлы…' : SS.num(r.files || 0) + ' ' + SS.pl(r.files || 0, ['файл', 'файла', 'файлов']));
  function rootRow(r, anim) {
    return h('div.root' + (anim ? '.rowin' : ''), { 'data-path': r.path },
      svg(ic.folder, { s: 16, sw: 1.6, cap: false, join: true }),
      h('span.p', { text: r.path }),
      h('span.n', { text: filesNote(r) }),
      h('button.xb', { type: 'button', 'aria-label': 'Убрать папку ' + r.path, 'data-root': r.path }, xIcon(14)));
  }
  function render() {
    // папки: существующие строки обновляются на месте, новая — разъезжается по высоте
    const roots = $('roots');
    const have = new Map(Array.from(roots.children).map((n) => [n.getAttribute('data-path'), n]));
    const list = S.content_roots || [];
    list.forEach((r) => {
      const n = have.get(r.path);
      if (n) { n.querySelector('.n').textContent = filesNote(r); have.delete(r.path); roots.appendChild(n); }
      else roots.appendChild(rootRow(r, r.path === fresh));
    });
    have.forEach((n) => { if (!n.classList.contains('rowout')) n.remove(); });
    // диски
    const drives = $('drives');
    drives.textContent = '';
    const sel = (S.drives && S.drives.selected) || [];
    ((S.drives && S.drives.available) || []).forEach((d) => {
      const on = sel.indexOf(d.drive) >= 0;
      drives.appendChild(h('button.chip', { type: 'button', 'aria-pressed': on ? 'true' : 'false', 'data-drive': d.drive,
        title: [d.label, d.fs].filter(Boolean).join(' · ') }, on ? svg(ic.check, { s: 12, sw: 2.6, join: true }) : null, d.drive + '\\'));
    });
    renderExcl([]);
    hotkeyCaps(S.hotkey);
    const names = { system: 'Как в Windows', light: 'Светлая', dark: 'Тёмная' };
    $('themeLbl').textContent = names[S.theme] || names.system;
    sw($('swFull'), !!S.pause_on_fullscreen);
    sw($('swBatt'), !!S.pause_on_battery);
    renderProcs('');
    document.querySelectorAll('.media input').forEach((inp) => {
      if (document.activeElement !== inp) inp.value = S[inp.getAttribute('data-key')] != null ? S[inp.getAttribute('data-key')] : '';
    });
    const i = S.info || {};
    $('iModel').textContent = (i.model || 'модель') + ' · ' + (i.device === 'cuda' ? 'на видеокарте' : i.device === 'cpu' ? 'на процессоре' : 'загружается');
    $('iSize').textContent = i.index_bytes != null ? SS.size(i.index_bytes) : '—';
    $('iRescan').textContent = every(i.rescan_minutes) + ', имена — ' + every(i.name_rescan_minutes);
    $('foreignLbl').textContent = FOREIGN[S.foreign_code || 'skip'] || FOREIGN.skip;
    $('iForeign').textContent = 'пропущено ' + SS.num(i.foreign_skipped || 0) + ' · копий ' + SS.num(i.copies || 0);
    const f = $('iFailed');
    f.textContent = '';
    if (i.failed) {
      f.append(SS.num(i.failed) + ' ' + SS.pl(i.failed, ['файл', 'файла', 'файлов']) + ' · ');
      f.appendChild(h('a', { href: '#log', id: 'why' }, 'почему'));
    } else f.textContent = 'нет';
  }
  function renderExcl(fresh2) {
    const ex = $('excl');
    ex.textContent = '';
    const list = S.exclude_dirs || [];
    list.slice(0, 8).forEach((x) => ex.appendChild(h('span' + (fresh2.indexOf(x) >= 0 ? '.pop' : ''), { text: x })));
    if (list.length > 8) ex.appendChild(h('span.more', { text: 'и ещё ' + (list.length - 8) }));
  }
  function renderProcs(added) {
    const procs = $('procs');
    procs.querySelectorAll('.proc').forEach((n) => n.remove());
    (S.pause_processes || []).forEach((p) => {
      procs.insertBefore(h('span.proc' + (p === added ? '.pop' : ''), { 'data-name': p }, p,
        h('button', { type: 'button', 'aria-label': 'Убрать ' + p, 'data-proc': p }, xIcon(12))), $('addProc'));
    });
  }
  const every = (m) => {
    m = +m || 0;
    if (!m) return 'вручную';
    if (m % 1440 === 0) return m === 1440 ? 'раз в сутки' : 'каждые ' + m / 1440 + ' дн.';
    if (m % 60 === 0) return m === 60 ? 'каждый час' : 'каждые ' + m / 60 + ' ч';
    return 'каждые ' + m + ' мин';
  };
  const sw = (b, on) => b.setAttribute('aria-checked', on ? 'true' : 'false');
  function hotkeyCaps(accel, stagger) {
    const k = $('hkKeys');
    k.textContent = '';
    SS.caps(accel).forEach((c, i) => {
      const kb = h('kbd.kb' + (stagger ? '.pop' : ''), { text: c });
      if (stagger) kb.style.animationDelay = i * 50 + 'ms';
      k.appendChild(kb);
    });
  }

  /* ---------- диалоги (Controls: 200 / 120 мс) ---------- */
  let dlgOpen = null;
  function dialog(o) {
    closeDialog(true);
    const prev = document.activeElement;
    const btns = o.buttons.map((b) => h('button.' + (b.danger ? 'prim.danger' : b.primary ? 'prim' : 'btn2'), { type: 'button' }, b.label));
    const dlg = h('div.dlg' + (o.wide ? '.wide' : ''), { role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'dlg-t' },
      h('div.hd', h('h3', { id: 'dlg-t', text: o.title }), o.text ? h('p', { text: o.text }) : null),
      o.body ? h('div.bd', o.body) : null,
      h('div.fbtns', btns));
    const ovl = h('div.ovl', dlg);
    o.buttons.forEach((b, i) => btns[i].addEventListener('click', async () => {
      if (b.onClick && (await b.onClick()) === false) return;
      closeDialog();
    }));
    ovl.addEventListener('mousedown', (e) => { if (e.target === ovl) closeDialog(); });
    $('win').appendChild(ovl);
    dlgOpen = { ovl, prev };
    const first = dlg.querySelector('textarea, input') || btns[0];
    if (first) first.focus();
    return dlg;
  }
  function closeDialog(now) {
    if (!dlgOpen) return;
    const { ovl, prev } = dlgOpen;
    dlgOpen = null;
    if (now) ovl.remove();
    else { ovl.classList.add('closing'); SS.after(ovl, 150, () => ovl.remove()); }
    if (prev && prev.focus) prev.focus();
  }

  /* ---------- папки ---------- */
  $('addRoot').addEventListener('click', async () => {
    let p = null;
    if (SS.tauri) {
      try { p = await SS.invoke('pick_folder'); } catch (e) { p = null; }
    } else p = window.prompt('Папка для поиска по смыслу:', '');
    if (!p) return;
    const cur = (S.content_roots || []).map((r) => r.path);
    if (cur.some((x) => x.toLowerCase() === p.toLowerCase())) { SS.toast('Эта папка уже в списке'); return; }
    fresh = p;
    if (await save({ content_roots: cur.concat([p]) })) {
      S.content_roots = (S.content_roots || []).concat([{ path: p, files: 0 }]);
      render();
      setTimeout(reload, 1500);
    }
  });
  $('roots').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-root]');
    if (!b) return;
    const p = b.getAttribute('data-root');
    const row = b.closest('.root');
    const cur = (S.content_roots || []).filter((r) => r.path !== p);
    row.classList.remove('rowin');
    row.classList.add('rowout');
    SS.after(row, 200, () => row.remove());
    if (await save({ content_roots: cur.map((r) => r.path) })) S.content_roots = cur;
    else reload();
  });

  /* ---------- диски ---------- */
  $('drives').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-drive]');
    if (!b) return;
    const d = b.getAttribute('data-drive');
    let sel = ((S.drives && S.drives.selected) || []).slice();
    const on = sel.indexOf(d) < 0;
    if (!on && sel.length === 1) { SS.toast('Нужен хотя бы один диск'); return; }
    sel = on ? sel.concat([d]) : sel.filter((x) => x !== d);
    b.setAttribute('aria-pressed', on ? 'true' : 'false');
    const old = b.querySelector('svg');
    if (old) old.remove();
    if (on) b.insertBefore(chkIcon(), b.firstChild);
    if (await save({ drives: sel })) S.drives.selected = sel;
    else render();
  });

  /* ---------- исключения ---------- */
  $('editExcl').addEventListener('click', () => {
    const ta = h('textarea', { spellcheck: 'false', 'aria-label': 'Папки, по одной в строке' });
    ta.value = (S.exclude_dirs || []).join('\n');
    dialog({ title: 'Не индексировать', text: 'По одному имени папки в строке. Такие папки пропускаются на любой глубине.', body: ta, wide: true,
      buttons: [{ label: 'Сохранить', primary: true, onClick: async () => {
        const seen = new Set();
        const list = ta.value.split(/\r?\n/).map((x) => x.trim()).filter((x) => x && !seen.has(x) && seen.add(x));
        const before = new Set(S.exclude_dirs || []);
        if (!(await save({ exclude_dirs: list }))) return false;
        renderExcl(list.filter((x) => !before.has(x)));
      } }, { label: 'Отмена' }] });
  });

  /* ---------- горячая клавиша: запись ---------- */
  let capturing = false;
  const MODS = ['Control', 'Alt', 'Shift', 'Meta', 'OS'];
  function recField(text) {
    const k = $('hkKeys');
    k.textContent = '';
    k.appendChild(h('span.rec', h('span.dot', { 'aria-hidden': 'true' }), h('span', { text: text })));
  }
  function stopCapture(accel, stagger) {
    capturing = false;
    $('hkBtn').textContent = 'Изменить';
    document.removeEventListener('keydown', onCaptureKey, true);
    hotkeyCaps(accel || S.hotkey, stagger);
  }
  function keyName(e) {
    const c = e.code || '';
    if (/^Key[A-Z]$/.test(c)) return c.slice(3).toLowerCase();
    if (/^Digit[0-9]$/.test(c)) return c.slice(5);
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(c)) return c.toLowerCase();
    const map = { Space: 'space', Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Backslash: '\\', Insert: 'insert', Home: 'home', End: 'end', PageUp: 'pageup', PageDown: 'pagedown' };
    return map[c] || '';
  }
  async function onCaptureKey(e) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape' && !e.ctrlKey && !e.altKey && !e.metaKey) { stopCapture(); return; }
    const mods = [];
    if (e.ctrlKey) mods.push('ctrl');
    if (e.altKey) mods.push('alt');
    if (e.shiftKey) mods.push('shift');
    if (e.metaKey) mods.push('win');
    if (MODS.indexOf(e.key) >= 0) { recField(mods.length ? SS.caps(mods.join('+')).join(' + ') + ' + …' : 'Нажмите сочетание…'); return; }
    const k = keyName(e);
    if (!k || !(e.ctrlKey || e.altKey || e.metaKey)) {
      $('hkErr').textContent = 'Нужно сочетание с Ctrl, Alt или Win — например, Ctrl+Alt+S';
      $('hkErr').hidden = false;
      return;
    }
    const accel = mods.concat([k]).join('+');
    document.removeEventListener('keydown', onCaptureKey, true);
    let err = null;
    if (SS.tauri) {
      try { err = await SS.invoke('set_hotkey', { accel }); } catch (x) { err = String(x || 'не удалось назначить'); }
    }
    if (err) {
      $('hkErr').textContent = 'Не получилось назначить ' + SS.caps(accel).join('+') + ': ' + err;
      $('hkErr').hidden = false;
      stopCapture();
      return;
    }
    $('hkErr').hidden = true;
    const ok = await save({ hotkey: accel });
    stopCapture(ok ? accel : null, ok);
  }
  $('hkBtn').addEventListener('click', () => {
    if (capturing) { stopCapture(); return; }
    capturing = true;
    $('hkErr').hidden = true;
    $('hkBtn').textContent = 'Отмена';
    recField('Нажмите сочетание…');
    document.addEventListener('keydown', onCaptureKey, true);
  });

  /* ---------- автозапуск ---------- */
  const auto = $('swAuto');
  if (SS.tauri) SS.invoke('get_autostart').then((v) => sw(auto, !!v)).catch(() => {});
  auto.addEventListener('click', async () => {
    const want = auto.getAttribute('aria-checked') !== 'true';
    sw(auto, want);
    if (!SS.tauri) { SS.toast('Сохранено'); return; }
    auto.disabled = true;
    try {
      await SS.invoke('set_autostart', { enabled: want });
      const v = !!(await SS.invoke('get_autostart'));
      sw(auto, v);
      SS.toast(v === want ? 'Сохранено' : 'Не удалось изменить автозапуск');
    } catch (e) {
      sw(auto, !want);
      SS.toast('Не удалось изменить автозапуск');
    } finally {
      auto.disabled = false;
    }
  });

  /* ---------- тема: список раскрывается от кнопки ---------- */
  const tbtn = $('themeBtn'), dd = tbtn.parentNode;
  let menuEl = null;
  function openMenu() {
    menuEl = h('div.menu', { role: 'listbox', 'aria-label': 'Тема' });
    [['light', 'Светлая'], ['dark', 'Тёмная'], ['system', 'Как в Windows']].forEach(([k, label]) => {
      const on = (S.theme || 'system') === k;
      menuEl.appendChild(h('button.opt', { type: 'button', role: 'option', 'aria-selected': on ? 'true' : 'false', 'data-theme': k },
        on ? h('span.bar', { 'aria-hidden': 'true' }) : null, label));
    });
    dd.appendChild(menuEl);
    tbtn.setAttribute('aria-expanded', 'true');
    (menuEl.querySelector('[aria-selected="true"]') || menuEl.querySelector('button')).focus();
  }
  function closeMenu(focusBtn) {
    if (!menuEl) return;
    const m = menuEl;
    menuEl = null;
    tbtn.setAttribute('aria-expanded', 'false');
    m.classList.add('closing');
    SS.after(m, 120, () => m.remove());
    if (focusBtn) tbtn.focus();
  }
  tbtn.addEventListener('click', () => (menuEl ? closeMenu() : openMenu()));
  dd.addEventListener('click', async (e) => {
    const b = e.target.closest('[data-theme]');
    if (!b || !menuEl) return;
    const t = b.getAttribute('data-theme');
    closeMenu(true);
    if (t === S.theme) return;
    if (await save({ theme: t })) { SS.applyTheme(t, S.accent); render(); }
  });
  dd.addEventListener('keydown', (e) => {
    if (!menuEl) return;
    const items = Array.from(menuEl.querySelectorAll('button'));
    const i = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length].focus();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeMenu(true);
    }
  });
  document.addEventListener('mousedown', (e) => { if (menuEl && !(e.target.closest && e.target.closest('.dd'))) closeMenu(); });

  /* ---------- чужой код: такой же список, как у темы ---------- */
  const fbtn = $('foreignBtn'), fdd = $('foreignDd');
  let fmenu = null;
  function openFMenu() {
    fmenu = h('div.menu', { role: 'listbox', 'aria-label': 'Чужой код и библиотеки' });
    Object.keys(FOREIGN).forEach((k) => {
      const on = (S.foreign_code || 'skip') === k;
      fmenu.appendChild(h('button.opt', { type: 'button', role: 'option', 'aria-selected': on ? 'true' : 'false', 'data-foreign': k },
        on ? h('span.bar', { 'aria-hidden': 'true' }) : null, FOREIGN[k]));
    });
    fdd.appendChild(fmenu);
    fbtn.setAttribute('aria-expanded', 'true');
    (fmenu.querySelector('[aria-selected="true"]') || fmenu.querySelector('button')).focus();
  }
  function closeFMenu(focusBtn) {
    if (!fmenu) return;
    const m = fmenu;
    fmenu = null;
    fbtn.setAttribute('aria-expanded', 'false');
    m.classList.add('closing');
    SS.after(m, 120, () => m.remove());
    if (focusBtn) fbtn.focus();
  }
  fbtn.addEventListener('click', () => (fmenu ? closeFMenu() : openFMenu()));
  fdd.addEventListener('click', async (e) => {
    const b = e.target.closest('[data-foreign]');
    if (!b || !fmenu) return;
    const v = b.getAttribute('data-foreign');
    closeFMenu(true);
    if (v === (S.foreign_code || 'skip')) return;
    if (await save({ foreign_code: v })) render();  // индекс пересчитает правило при ближайшем обходе
  });
  fdd.addEventListener('keydown', (e) => {
    if (!fmenu) return;
    const items = Array.from(fmenu.querySelectorAll('button'));
    const i = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length].focus();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeFMenu(true);
    }
  });
  document.addEventListener('mousedown', (e) => { if (fmenu && !(e.target.closest && e.target.closest('#foreignDd'))) closeFMenu(); });

  /* ---------- паузы и процессы ---------- */
  $('swFull').addEventListener('click', async () => {
    const v = $('swFull').getAttribute('aria-checked') !== 'true';
    sw($('swFull'), v);
    if (!(await save({ pause_on_fullscreen: v }))) sw($('swFull'), !v);
  });
  $('swBatt').addEventListener('click', async () => {
    const v = $('swBatt').getAttribute('aria-checked') !== 'true';
    sw($('swBatt'), v);
    if (!(await save({ pause_on_battery: v }))) sw($('swBatt'), !v);
  });
  $('procs').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-proc]');
    if (!b) return;
    const p = b.getAttribute('data-proc');
    const chip = b.closest('.proc');
    chip.classList.add('shrink');
    SS.after(chip, 180, () => chip.remove());
    const rest = (S.pause_processes || []).filter((x) => x !== p);
    if (!(await save({ pause_processes: rest }))) renderProcs('');
  });
  $('addProc').addEventListener('click', async () => {
    const inp = h('input', { type: 'text', placeholder: 'Имя процесса, например game.exe', spellcheck: 'false', 'aria-label': 'Имя процесса' });
    const list = h('div.plist', h('p', { text: 'Загружаю список запущенных программ…' }));
    const add = async (name) => {
      name = String(name || '').trim();
      if (!name) return false;
      if (!/\.exe$/i.test(name)) name += '.exe';
      const cur = S.pause_processes || [];
      if (cur.some((x) => x.toLowerCase() === name.toLowerCase())) return true;
      if (!(await save({ pause_processes: cur.concat([name]) }))) return false;
      renderProcs(name);
      return true;
    };
    let all = [];
    const fill = () => {
      const f = inp.value.trim().toLowerCase();
      const have = new Set((S.pause_processes || []).map((x) => x.toLowerCase()));
      list.textContent = '';
      const shown = all.filter((p) => !have.has(p.toLowerCase()) && (!f || p.toLowerCase().indexOf(f) >= 0));
      shown.slice(0, 200).forEach((p) => list.appendChild(h('button', { type: 'button', 'data-p': p }, p)));
      if (!shown.length) list.appendChild(h('p', { text: f ? 'Нажмите «Добавить», чтобы добавить «' + inp.value.trim() + '».' : 'Запущенных программ не нашлось.' }));
    };
    list.addEventListener('click', async (e) => {
      const b = e.target.closest('[data-p]');
      if (b && (await add(b.getAttribute('data-p')))) closeDialog();
    });
    inp.addEventListener('input', fill);
    inp.addEventListener('keydown', async (e) => {
      if (e.key === 'Enter') { e.preventDefault(); if (await add(inp.value)) closeDialog(); }
    });
    dialog({ title: 'Пауза, пока запущен процесс', text: 'Выберите запущенную программу или впишите имя .exe.', body: [inp, list], wide: true,
      buttons: [{ label: 'Добавить', primary: true, onClick: () => add(inp.value) }, { label: 'Отмена' }] });
    try {
      const r = await SS.api('/api/processes');
      all = Array.from(new Set((r.items || []).map(String))).sort((a, b) => a.localeCompare(b));
    } catch (e) { all = []; }
    fill();
  });

  /* ---------- числа ---------- */
  document.querySelectorAll('.media input').forEach((inp) => {
    inp.addEventListener('change', async () => {
      const k = inp.getAttribute('data-key');
      let v = Math.round(+inp.value);
      if (!isFinite(v) || v < 1) v = S[k] || 1;
      inp.value = v;
      if (v !== S[k]) await save({ [k]: v });
    });
    inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') inp.blur(); });
  });

  /* ---------- индекс: долгое действие «Пересканировать сейчас» ---------- */
  let scanBusy = false;
  const bScan = $('bRescan');
  function scanState(kind, label) {
    bScan.textContent = '';
    if (kind === 'busy') {
      const sp = svg('M12 3a9 9 0 0 1 9 9', { s: 14, sw: 2.6, cls: 'spin' });
      bScan.appendChild(sp);
    } else if (kind === 'done') {
      const c = svg(ic.check, { s: 14, sw: 2.6, join: true });
      c.firstChild.setAttribute('class', 'chk');
      bScan.appendChild(c);
    }
    bScan.appendChild(h('span', { text: label }));
  }
  bScan.addEventListener('click', async () => {
    if (scanBusy) return;
    scanBusy = true;
    scanState('busy', 'Сканирую…');
    const t0 = Date.now();
    let res = null, ok = true;
    try { res = await SS.api('/api/rescan', { body: {} }); } catch (e) { ok = false; }
    for (let i = 0; ok && i < 30; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      let st = null;
      try { st = await SS.api('/api/status'); } catch (e) { break; }
      if (!(st && st.state === 'indexing' && st.phase === 'scan') && Date.now() - t0 > 900) break;
    }
    const n = res && typeof res.new_files === 'number' ? res.new_files : null;
    scanState(ok ? 'done' : '', ok ? (n === null ? 'Готово' : 'Готово, новых файлов: ' + SS.num(n)) : 'Не удалось запустить');
    setTimeout(() => { scanState('', 'Пересканировать сейчас'); scanBusy = false; }, 2000);
  });
  $('bLog').addEventListener('click', async () => {
    try { await SS.api('/api/open_log', { body: {} }); } catch (e) { SS.toast('Не удалось открыть журнал'); }
  });
  $('bRebuild').addEventListener('click', () => {
    dialog({ title: 'Собрать индекс заново?', text: 'Текущий индекс удалится и соберётся с нуля.',
      buttons: [{ label: 'Собрать заново', danger: true, onClick: async () => {
        try { await SS.api('/api/rebuild', { body: {} }); SS.toast('Индекс собирается заново'); } catch (e) { SS.toast('Не удалось начать сборку'); return false; }
      } }, { label: 'Отмена' }] });
  });
  $('iFailed').addEventListener('click', async (e) => {
    if (!e.target.closest('#why')) return;
    e.preventDefault();
    const box = h('div.flist', h('p', { text: 'Загружаю…' }));
    const dlg = dialog({ title: 'Не удалось прочитать', text: 'Подробности — в журнале.', body: box, wide: true, buttons: [{ label: 'Закрыть', primary: true }] });
    try {
      const r = await SS.api('/api/failed?limit=200');
      dlg.querySelector('.hd p').textContent = SS.num(r.total) + ' ' + SS.pl(r.total, ['файл', 'файла', 'файлов']) +
        (r.total > (r.items || []).length ? ', показаны первые ' + r.items.length : '') + '. Подробности — в журнале.';
      box.textContent = '';
      (r.items || []).forEach((x) => box.appendChild(h('div', h('div.fp', { text: x.path }), h('div.fe', { text: x.err }))));
    } catch (x) {
      box.textContent = '';
      box.appendChild(h('p', { text: 'Не удалось получить список.' }));
    }
  });

  /* ---------- окно ---------- */
  if (SS.tauri || /caps/.test(location.hash)) {
    $('caps').hidden = false;
    $('capMin').addEventListener('click', () => SS.invoke('win_minimize').catch(() => {}));
    $('capClose').addEventListener('click', () => SS.invoke('win_hide').catch(() => {}));
  }
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || e.defaultPrevented || capturing) return;
    if (dlgOpen) { e.preventDefault(); closeDialog(); return; }
    if (menuEl || fmenu) return;
    if (SS.tauri) SS.invoke('win_hide').catch(() => {});
  });
  document.addEventListener('visibilitychange', () => { if (!document.hidden && !dlgOpen && !capturing) reload(); });

  reload();
})();

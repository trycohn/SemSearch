/* SemSearch — главное окно: состояние, поиск, выбор, клавиши, статус, анимации окна. */
(function () {
  'use strict';
  const { h, svg, ic } = SS;
  const $ = (id) => document.getElementById(id);
  const el = {};
  ['win', 'caps', 'capMin', 'capMax', 'capHide', 'sbox', 'ibar', 'simchip', 'q', 'clr', 'ssep', 'imgb', 'imgfile', 'tabs', 'main',
    'results', 'pane', 'start', 'chips', 'fix', 'fixRows', 'eta', 'kgrid', 'dropovl', 'st', 'mid', 'pauseb', 'pauseLbl', 'setb',
    'boot', 'bLogo', 'bDot', 'bErrIco', 'bTitle', 'bSub', 'bBar', 'bFill', 'bSteps', 'bErr', 'bRetry', 'bLog', 'bNames', 'bByName', 'bByName2', 'bFoot'
  ].forEach((id) => (el[id] = $(id)));

  const devCaps = /caps/.test(location.hash);
  const S = {
    q: '', tab: 'all', mode: 'text', simChunk: null, imgFile: null, imgKey: 0, sim: null, autoTab: false,
    resp: null, counts: null, flat: [], sel: -1, err: 0, errText: '',
    status: null, settings: null, pv: null
  };

  /* ---------- окно: запуск 320 мс, показ 160 мс, скрытие 120 мс (сначала анимация, потом win_hide) ---------- */
  const winDone = (e) => { if (e.target === el.win) el.win.classList.remove('w-launch', 'w-in'); };
  el.win.addEventListener('animationend', winDone);
  /* окно создано скрытым (автозапуск): первая показ — как запуск */
  let firstShow = document.hidden, wasHidden = document.hidden, shownAt = -1e9;
  if (firstShow) el.win.classList.replace('w-launch', 'w-pre');
  function animateIn() {
    if (performance.now() - shownAt < 400) {
      /* повтор (focus-search + visibilitychange) — анимация уже идёт, но окно не должно остаться невидимым */
      el.win.classList.remove('w-pre', 'w-out');
      return;
    }
    shownAt = performance.now();
    const cls = firstShow ? 'w-launch' : 'w-in';
    firstShow = false;
    wasHidden = false;
    el.win.classList.remove('w-out', 'w-pre', 'w-launch', 'w-in');
    void el.win.offsetWidth;
    el.win.classList.add(cls);
  }
  let hiding = false;
  function hideWindow() {
    if (!SS.tauri) return;
    if (hiding) return;
    if (SS.reduced()) { SS.invoke('win_hide').catch(() => {}); return; }
    hiding = true;
    el.win.classList.remove('w-launch', 'w-in');
    el.win.classList.add('w-out');
    SS.after(el.win, 120, () => {
      hiding = false;
      SS.invoke('win_hide').catch(() => el.win.classList.remove('w-out'));
    });
  }

  /* ---------- поиск ---------- */
  const cache = new Map();
  const remember = (m, k, v, max) => { m.delete(k); m.set(k, v); while (m.size > max) m.delete(m.keys().next().value); };
  const isStart = () => S.mode === 'text' && !S.q.trim();
  const baseKey = () => S.mode + '\n' + (S.mode === 'text' ? S.q.trim() : S.mode === 'similar' ? S.simChunk : S.imgKey);
  const MEDIA_TABS = ['all', 'image', 'video'];
  let ctl = null, debT = 0, seq = 0, slowT = 0;

  const schedule = (ms) => { clearTimeout(debT); debT = setTimeout(run, ms); };
  const request = (tab, signal) => {
    if (S.mode === 'text') return SS.api('/api/search?' + SS.q({ q: S.q.trim(), tab }), { signal });
    if (S.mode === 'similar') return SS.api('/api/similar?' + SS.q({ chunk: S.simChunk, tab }), { signal });
    const fd = new FormData();
    fd.append('file', S.imgFile, S.imgFile.name || 'image.png');
    return SS.api('/api/search_image?' + SS.q({ tab }), { method: 'POST', form: fd, signal });
  };

  /* ожидание дольше 150 мс: бегущая полоска под полем; старая выдача бледнеет до 60%, без неё — скелетоны */
  function slow(on) {
    clearTimeout(slowT);
    if (!on) { el.ibar.hidden = true; return; }
    slowT = setTimeout(() => {
      el.ibar.hidden = false;
      const kids = Array.from(el.results.children);
      if (S.flat.length && kids.length) kids.forEach((k) => k.classList.add('old'));
      else {
        el.results.className = 'results';
        el.results.textContent = '';
        el.results.appendChild(R.skel(S.tab));
        S.flat = [];
        S.sel = -1;
        R.pane(el.pane, { it: null, skel: true });
      }
      if (!el.tabs.hidden) R.pill(el.tabs, S.tab);
    }, 150);
  }

  async function run() {
    chrome();
    if (isStart()) { if (ctl) ctl.abort(); slow(false); return; }
    const key = baseKey() + '\n' + S.tab;
    if (S.mode !== 'text' && MEDIA_TABS.indexOf(S.tab) < 0) {
      show({ counts: S.counts || {}, items: [], ms: 0 }, true);
      return;
    }
    if (cache.has(key)) { S.err = 0; slow(false); show(cache.get(key)); return; }
    if (ctl) ctl.abort();
    ctl = new AbortController();
    const my = ++seq;
    slow(true);
    try {
      const resp = await request(S.tab, ctl.signal);
      if (my !== seq) return;
      remember(cache, key, resp, 40);
      S.err = 0;
      slow(false);
      show(resp);
    } catch (e) {
      if (e.name === 'AbortError' || my !== seq) return;
      slow(false);
      S.err = e.status === 503 ? 503 : e.status || 1;
      S.errText = e.detail || '';
      show(null);
    }
  }

  function show(resp, synthetic) {
    if (resp && !synthetic) S.counts = resp.counts || S.counts;
    if (resp && S.autoTab && S.mode !== 'text') {
      S.autoTab = false;
      const c = resp.counts || {};
      if (S.tab === 'image' && !c.image && c.video) { setTab('video'); return; }
    }
    S.resp = resp;
    const items = (resp && resp.items) || [];
    R.tabs(el.tabs, S.counts, S.tab, resp && items.length ? R.metaLine(S.tab, resp) : '');
    R.pill(el.tabs, S.tab, !items.length);
    S.flat = R.results(el.results, {
      tab: S.tab, resp: resp || { items: [], counts: S.counts }, re: S.mode === 'text' ? R.hlRe(S.q) : null,
      error: S.err, errorText: S.errText, videoNote: videoNote(), status: S.status, settings: S.settings, anim: true
    });
    el.results.scrollTop = 0;
    select(S.flat.length ? 0 : -1, true);
  }

  /* «Ещё N видео…» под сеткой фото */
  function videoNote() {
    const c = S.counts || {};
    if (S.tab !== 'image' || !c.video || !S.resp || !(S.resp.items || []).length) return null;
    const base = baseKey();
    const src = cache.get(base + '\nvideo') || cache.get(base + '\nall');
    if (!src && S.mode !== 'image') {
      request('video').then((r) => {
        remember(cache, base + '\nvideo', r, 40);
        if (baseKey() !== base || S.tab !== 'image') return;
        const old = el.results.querySelector('.vnote');
        const nu = videoNote();
        if (old && nu) old.replaceWith(nu);
      }).catch(() => {});
    }
    const vids = src ? (src.items || []).filter((i) => i.kind === 'video') : [];
    return R.videoNote(S.mode === 'text' ? S.q.trim() : '', vids, c.video);
  }

  function setTab(t) {
    if (!t || S.tab === t) return;
    S.tab = t;
    if (isStart()) return;
    el.tabs.querySelectorAll('[data-tab]').forEach((b) => b.setAttribute('aria-selected', b.getAttribute('data-tab') === t ? 'true' : 'false'));
    R.pill(el.tabs, t);
    schedule(0);
  }

  /* картинка-запрос: чип «похоже на …» в поле */
  function setChip(sim) {
    if (S.sim && S.sim.blob) URL.revokeObjectURL(S.sim.thumb);
    S.sim = sim;
    el.simchip.textContent = '';
    el.simchip.hidden = !sim;
    if (!sim) return;
    el.simchip.append(
      sim.thumb ? h('img.sim-th', { src: sim.thumb, alt: '' }) : h('span.sim-th'),
      h('span.sim-t', { text: SS.t('img.similar', { name: sim.label || SS.t('img.anImage') }), title: sim.label || '' }),
      h('button', { type: 'button', 'aria-label': SS.t('img.remove') }, svg(ic.x, { s: 10, sw: 2.4 })));
    el.simchip.classList.remove('pop');
    void el.simchip.offsetWidth;
    el.simchip.classList.add('pop');
  }
  function enterMode(mode, o) {
    S.mode = mode;
    S.simChunk = o.chunk === undefined ? null : o.chunk;
    S.imgFile = o.file || null;
    if (o.file) S.imgKey++;
    setChip({ label: o.label, thumb: o.thumb || '', blob: !!o.blob });
    S.q = '';
    el.q.value = '';
    el.q.placeholder = '';
    S.tab = 'image';
    S.autoTab = true;
    S.counts = null;
    S.flat = [];
    schedule(0);
  }
  function exitMode() {
    if (S.mode === 'text') return;
    S.mode = 'text';
    S.imgFile = null;
    S.simChunk = null;
    S.counts = null;
    setChip(null);
    el.q.placeholder = SS.t('q.ph');
  }
  function clearQuery() {
    exitMode();
    S.q = '';
    el.q.value = '';
    if (ctl) ctl.abort();
    slow(false);
    S.flat = [];
    chrome();
    el.q.focus();
  }

  /* ---------- выбор и просмотр (фон 150 мс, метка 200 мс, наплыв 220 мс) ---------- */
  function select(i, fresh) {
    const old = el.results.querySelector('.sel[data-i]');
    if (old) { old.classList.remove('sel'); old.setAttribute('aria-pressed', 'false'); }
    S.sel = i;
    const node = i >= 0 ? el.results.querySelector('[data-i="' + i + '"]') : null;
    if (node) {
      node.classList.add('sel');
      node.setAttribute('aria-pressed', 'true');
      if (!fresh) node.scrollIntoView({ block: 'nearest' });
    }
    loadPreview(fresh);
  }
  function move(d) {
    if (!S.flat.length) return;
    select(Math.max(0, Math.min(S.flat.length - 1, (S.sel < 0 ? -1 : S.sel) + d)));
  }

  const pvCache = new Map();
  let pvCtl = null, pvT = 0, pvSlow = 0, painted = null;
  function loadPreview(fresh) {
    const it = S.flat[S.sel];
    clearTimeout(pvT);
    clearTimeout(pvSlow);
    if (pvCtl) pvCtl.abort();
    painted = null;
    if (!it) { S.pv = null; R.pane(el.pane, { it: null }); return; }
    const chunks = (it.hits || []).map((x) => x.chunk).filter((x) => x !== null && x !== undefined);
    const key = it.path + '\n' + chunks.join(',');
    if (pvCache.has(key)) { paint(it, pvCache.get(key), true); return; }
    pvSlow = setTimeout(() => { if (S.flat[S.sel] === it) paint(it, null, true); }, 150);
    pvT = setTimeout(() => {
      pvCtl = new AbortController();
      SS.api('/api/preview?' + SS.q({ path: it.path, chunks: chunks.join(',') }), { signal: pvCtl.signal })
        .then((pv) => {
          remember(pvCache, key, pv, 80);
          if (S.flat[S.sel] === it) { clearTimeout(pvSlow); paint(it, pv, painted !== it); }
        })
        .catch((e) => {
          if (e.name === 'AbortError') return;
          if (S.flat[S.sel] === it) { clearTimeout(pvSlow); if (painted !== it) paint(it, null, true); }
        });
    }, fresh ? 0 : 40);
  }
  function paint(it, pv, anim) {
    S.pv = pv;
    painted = it;
    R.pane(el.pane, { it, pv, tab: S.tab, anim });
    el.pane.scrollTop = 0;
  }

  async function act(name) {
    const it = S.flat[S.sel];
    if (!it) return;
    if (name === 'open' || name === 'reveal') {
      try {
        await SS.api('/api/open', { body: { path: it.path, reveal: name === 'reveal' } });
      } catch (e) {
        SS.toast(SS.t(e.status === 404 ? 'toast.notFound' : 'toast.openFail'));
      }
    } else if (name === 'copy') {
      SS.toast(SS.t((await SS.copy(it.path)) ? 'toast.copied' : 'toast.copyFail'));
    } else if (name === 'similar') {
      const kind = S.pv && S.pv.kind ? S.pv.kind : it.kind;
      const can = S.pv ? !!S.pv.can_similar : kind === 'image' || kind === 'video';
      const hit = (it.hits || [])[0];
      if (!can || S.tab === 'name' || !hit || hit.chunk === null || hit.chunk === undefined) return;
      enterMode('similar', { chunk: hit.chunk, label: it.name, thumb: SS.thumbUrl(it.thumb, 360) });
    }
  }
  function searchImage(f) {
    if (!f) return;
    if (!/^image\//.test(f.type || '')) { SS.toast(SS.t('img.need')); return; }
    const named = f.name && f.name !== 'image.png';
    enterMode('image', { file: f, label: named ? f.name : '', thumb: URL.createObjectURL(f), blob: true });
  }

  /* ---------- шапка/подвал/старт ---------- */
  function chrome() {
    const st = isStart();
    el.start.hidden = !st;
    el.tabs.hidden = st;
    el.main.hidden = st;
    el.ssep.hidden = st;
    el.clr.hidden = st || !S.q;
    if (!st) R.tabs(el.tabs, S.counts, S.tab, S.resp && S.flat.length ? R.metaLine(S.tab, S.resp) : '');
    footer();
    if (st) startScreen();
  }
  let footerPainted = false;
  function footer() {
    const s = S.status;
    const nameTab = !isStart() && S.tab === 'name';
    R.paintStatus(el.st, nameTab ? R.namesStatus(s) : R.status(s), { quiet: !footerPainted });
    footerPainted = true;
    const n = s && s.names;
    el.mid.textContent = isStart() ? R.device(s)
      : nameTab ? SS.t(n && n.drives && n.drives.length > 1 ? 'mid.allDrives' : 'mid.oneDrive')
      : R.summary(s);
    el.pauseb.hidden = !s;
    const manual = !!(s && s.paused && s.paused.reason === 'manual');
    el.pauseb.classList.toggle('paused', manual);
    el.pauseLbl.textContent = SS.t(manual ? 'foot.resume' : 'foot.pause');
    el.sbox.classList.toggle('dim', !!(s && !s.ready));
  }
  let keysFor = null;
  function startScreen() {
    const s = S.status;
    const pend = R.pending(s);
    el.fix.hidden = !pend;
    if (pend) { R.fixRows(el.fixRows, s); el.eta.textContent = R.eta(s.eta_sec); }
    const hk = (S.settings && S.settings.hotkey) || 'ctrl+alt+s';
    if (keysFor !== hk) { keysFor = hk; R.keys(el.kgrid, hk, !!SS.tauri || devCaps); }
  }

  /* ---------- экран запуска (design/Launch): шаги из /api/status → boot, пока модель не готова ---------- */
  const BOOT_W = { index: 4, download: 56, model: 26, vectors: 9, watch: 5 };
  const boot = { on: true, leaving: false, ready: false, keys: '', step: null, stepAt: 0, hk: '' };
  const bootKeys = (s) => ['index'].concat(s && s.boot && s.boot.first_run ? ['download'] : [], ['model', 'vectors', 'watch']);
  /* «на видеокарте» / «на процессоре» — в пометке справа, а не в названии шага: иначе оно повторяется дважды */
  const bootLabel = (k) => SS.t('boot.s.' + k);
  function bootNote(k, state, s) {
    const b = (s && s.boot) || {};
    if (k === 'index' && state === 'done') {
      const n = s && s.kinds ? Object.keys(s.kinds).reduce((a, x) => a + ((s.kinds[x] && s.kinds[x].total) || 0), 0) : 0;
      return n ? SS.tn('boot.n.files', n) : '';
    }
    if (k === 'download') {
      const d = b.download;
      if (state === 'active' && d) return d.total ? SS.t('boot.n.of', { a: SS.size(d.done || 0), b: SS.size(d.total) }) : SS.t('boot.n.got', { a: SS.size(d.done || 0) });
      return state === 'done' ? (d && d.total ? SS.size(d.total) : '') : SS.t('boot.n.once');
    }
    if (k === 'model' && state === 'done') {
      if (s.device === 'cuda') return SS.t('boot.n.gpu');
      return SS.t(b.gpu_note ? 'boot.n.' + b.gpu_note : 'boot.n.cpu');
    }
    if (k === 'vectors' && state === 'done' && typeof b.chunks === 'number') return SS.tn('boot.n.chunks', b.chunks);
    return '';
  }
  function bootIcon(state) {
    if (state === 'done') return R.check(16, 2.4);
    if (state === 'active') return R.ring(16, 2.6);
    if (state === 'failed') return svg(ic.x, { s: 16, sw: 2.4, cls: 'pop' });
    return h('span.wait');
  }
  function bootPaint() {
    if (!boot.on) return;
    const s = S.status;
    const b = (s && s.boot) || {};
    const err = !!(s && !s.ready && (s.state === 'error' || s.error));
    const ready = !!(s && s.ready);
    const keys = bootKeys(s);
    let idx = ready ? keys.length : s ? keys.indexOf(b.step) : 0;
    if (idx < 0 && err) idx = keys.indexOf(b.error_kind === 'download' && keys.indexOf('download') >= 0 ? 'download' : 'model');
    if (idx < 0) idx = 0;
    if (b.step !== boot.step) { boot.step = b.step; boot.stepAt = performance.now(); }
    /* строки: пересобираются, только если сменился сам список (появился шаг «скачиваю модель») */
    const sig = keys.join(',') + '|' + SS.lang();
    if (sig !== boot.keys) {
      const fresh = !boot.keys;
      boot.keys = sig;
      el.bSteps.textContent = '';
      keys.forEach((k, i) => {
        const row = h('div.bstep' + (fresh ? '.rs' : ''), { 'data-k': k }, h('span.bi'), h('span.bl'), h('span.bn'));
        if (fresh) row.style.animationDelay = 250 + i * 60 + 'ms';
        el.bSteps.appendChild(row);
      });
    }
    Array.from(el.bSteps.children).forEach((row, i) => {
      const k = row.getAttribute('data-k');
      const state = i < idx ? 'done' : i === idx ? (err ? 'failed' : 'active') : 'wait';
      if (row.getAttribute('data-s') !== state) {
        row.setAttribute('data-s', state);
        row.className = 'bstep ' + state + (row.classList.contains('rs') ? ' rs' : '');
        const bi = row.querySelector('.bi');
        bi.textContent = '';
        bi.appendChild(bootIcon(state));
      }
      row.querySelector('.bl').textContent = bootLabel(k, s);
      row.querySelector('.bn').textContent = err && state === 'failed' && b.error_kind === 'download' ? '' : bootNote(k, state, s);
    });
    el.bLogo.hidden = err;
    el.bErrIco.hidden = !err;
    el.bDot.setAttribute('class', 'bdot ' + (ready ? 'settle' : 'orbit'));
    el.bTitle.textContent = err ? SS.t(b.error_kind === 'download' ? 'boot.errDownload' : 'boot.errOther')
      : ready ? SS.t('boot.ready') : SS.t(b.first_run ? 'boot.firstRun' : 'boot.title');
    el.bSub.textContent = err ? SS.t(b.error_kind === 'download' ? 'boot.errDownloadNote' : 'boot.errOtherNote')
      : ready ? SS.t(b.first_run ? 'boot.openingFirst' : 'boot.opening')
      : s ? bootLabel(keys[idx], s) + '…' : SS.t('boot.starting');
    el.bSub.title = err && s.error ? s.error : '';
    el.bBar.hidden = err;
    el.bErr.hidden = !err;
    el.bNames.hidden = ready || err;
    /* полоса: сделанные шаги + доля текущего (скачивание — по байтам, остальное ползёт к 90% своей доли) */
    const sum = keys.reduce((a, k) => a + BOOT_W[k], 0);
    let pct = 100;
    if (!ready) {
      const base = keys.slice(0, idx).reduce((a, k) => a + BOOT_W[k], 0);
      const cur = keys[idx];
      const d = b.download;
      const frac = cur === 'download' && d && d.total ? Math.min(0.98, (d.done || 0) / d.total)
        : Math.min(0.9, 1 - Math.exp(-(performance.now() - boot.stepAt) / 7000));
      pct = Math.round(((base + BOOT_W[cur] * frac) / sum) * 100);
    }
    el.bFill.style.width = pct + '%';
    el.bBar.setAttribute('aria-valuenow', String(pct));
    const hk = SS.caps((S.settings && S.settings.hotkey) || 'ctrl+alt+s').join('+');
    el.bFoot.textContent = SS.tauri ? SS.t('boot.foot', { hk: hk }) : '';
    el.boot.classList.remove('pre');
    if (ready && !boot.ready) {
      boot.ready = true;
      setTimeout(() => bootLeave(false), SS.reduced() ? 0 : 900);
    }
  }
  /* экран запуска уходит вверх (300 мс), а окно поиска под ним в это же время проявляется из прозрачного.
     Пока экран запуска на месте, окно поиска скрыто (.booting): иначе при уходе оно сначала просвечивало бы
     целиком, а потом его собственная анимация начиналась бы с нуля — двойное мигание. */
  const under = () => [document.querySelector('.sbar'), $('body'), $('ft')].filter(Boolean);
  el.win.classList.add('booting');
  function bootLeave(byName) {
    if (!boot.on || boot.leaving) return;
    boot.leaving = true;
    if (byName) S.tab = 'name';
    chrome();
    const done = () => {
      boot.on = false;
      el.boot.hidden = true;
      el.boot.classList.remove('leave');
      el.q.focus();
    };
    el.win.classList.remove('booting');
    if (SS.reduced()) { done(); return; }
    under().forEach((x) => {
      x.classList.add('reveal');
      SS.after(x, 320, () => x.classList.remove('reveal'));
    });
    el.boot.classList.add('leave');
    SS.after(el.boot, 300, done);
  }
  function bootSkip() {
    boot.on = false;
    el.boot.hidden = true;
    el.win.classList.remove('booting');
  }
  el.bByName.addEventListener('click', () => bootLeave(true));
  /* пришли с заставки оболочки (#boot): экран запуска продолжает её кадр, без повторного появления */
  if (/(^|[#&])boot($|&)/.test(location.hash)) {
    el.win.classList.remove('w-launch');
    el.boot.classList.add('calm');
    bootPaint();
  }
  el.bByName2.addEventListener('click', () => bootLeave(true));
  el.bRetry.addEventListener('click', async () => {
    try { await SS.api('/api/boot/retry', { body: {} }); } catch (e) { if (e.status !== 409) SS.toast(SS.t('boot.retryFail')); }
    pollStatus();
  });
  el.bLog.addEventListener('click', () => { SS.api('/api/open_log', { body: {} }).catch(() => {}); });

  /* ---------- статус (раз в 1 с во время индексации, иначе 2 с; только пока окно видно) ---------- */
  let stT = 0, stBusy = false, readyT = 0;
  async function pollStatus() {
    clearTimeout(stT);
    if (document.hidden) return;
    if (!stBusy) {
      stBusy = true;
      try {
        S.status = await SS.api('/api/status');
        if (S.err === 503 && S.status.ready && !isStart() && !readyT) modelReady();
      } catch (e) {
        S.status = null;
      } finally {
        stBusy = false;
      }
      /* окно открыли, когда модель уже готова, — экрана запуска нет вовсе */
      if (boot.on && !boot.ready && !boot.keys && S.status && S.status.ready) bootSkip();
      else bootPaint();
      footer();
      syncMload();
      if (isStart()) startScreen();
    }
    const loading = !S.status || !S.status.ready;
    const busy = S.status && (S.status.state === 'indexing' || S.status.state === 'loading' || (S.status.names && !S.status.names.ready));
    stT = setTimeout(pollStatus, boot.on && loading ? 400 : busy ? 1000 : 2000);
  }
  /* запуск упал, а экран запуска пропустили: «Повторить» прямо в результатах, плашка сама сменится на загрузку */
  async function retryBoot(btn) {
    btn.disabled = true;
    try { await SS.api('/api/boot/retry', { body: {} }); } catch (e) { if (e.status !== 409) SS.toast(SS.t('boot.retryFail')); }
    btn.disabled = false;
    await pollStatus();
  }
  function syncMload() {
    const box = el.results.querySelector('.mload:not([data-done])');
    if (box && box.hasAttribute('data-fail') !== !!R.bootFail(S.status)) box.replaceWith(R.modelLoading(S.status, S.settings, false));
  }
  /* модель догрузилась: галочка «Готово — можно искать», затем повтор запроса */
  function modelReady() {
    const box = el.results.querySelector('.mload');
    if (box) box.replaceWith(R.modelLoading(S.status, S.settings, true));
    readyT = setTimeout(() => {
      readyT = 0;
      cache.clear();
      schedule(0);
    }, SS.reduced() ? 0 : 650);
  }
  async function loadSettings() {
    try {
      S.settings = await SS.api('/api/settings');
      SS.applyTheme(S.settings.theme, S.settings.accent);
      SS.setLang(S.settings.lang, S.settings.lang_system);
      if (isStart()) startScreen();
      bootPaint();
    } catch (e) { /* backend ещё не готов */ }
  }

  /* ---------- события ---------- */
  el.q.addEventListener('input', () => {
    S.q = el.q.value;
    exitMode();
    cache.forEach((v, k) => { if (k.indexOf('text\n') === 0 && k.split('\n')[1] !== S.q.trim()) cache.delete(k); });
    if (isStart()) { clearTimeout(debT); if (ctl) ctl.abort(); slow(false); S.flat = []; chrome(); return; }
    const showClr = !!S.q;
    if (el.clr.hidden === showClr) {
      el.clr.hidden = !showClr;
      if (showClr) { el.clr.classList.remove('pop'); void el.clr.offsetWidth; el.clr.classList.add('pop'); }
    }
    if (el.main.hidden) chrome();
    schedule(60);
  });
  el.clr.addEventListener('click', clearQuery);
  el.simchip.addEventListener('click', (e) => { if (e.target.closest('button')) clearQuery(); });
  el.imgb.addEventListener('click', () => el.imgfile.click());
  el.imgfile.addEventListener('change', () => {
    const f = el.imgfile.files && el.imgfile.files[0];
    el.imgfile.value = '';
    searchImage(f);
  });
  el.chips.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    el.q.value = b.textContent;
    el.q.dispatchEvent(new Event('input'));
    el.q.focus();
  });
  el.tabs.addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]');
    if (b) setTab(b.getAttribute('data-tab'));
  });
  el.results.addEventListener('click', (e) => {
    const rb = e.target.closest('[data-retry]');
    if (rb) { retryBoot(rb); return; }
    const go = e.target.closest('[data-go]');
    if (go) {
      let t = go.getAttribute('data-go');
      const c = S.counts || {};
      if (t === 'image' && !c.image && c.video) t = 'video';
      setTab(t);
      return;
    }
    const b = e.target.closest('[data-i]');
    if (b) select(+b.getAttribute('data-i'));
  });
  el.results.addEventListener('dblclick', (e) => {
    const b = e.target.closest('[data-i]');
    if (b) { select(+b.getAttribute('data-i')); act('open'); }
  });
  el.pane.addEventListener('click', (e) => {
    const b = e.target.closest('[data-act]');
    if (b) act(b.getAttribute('data-act'));
  });
  el.pauseb.addEventListener('click', async () => {
    const manual = !!(S.status && S.status.paused && S.status.paused.reason === 'manual');
    el.pauseb.classList.toggle('paused', !manual);
    el.pauseLbl.textContent = SS.t(manual ? 'foot.pause' : 'foot.resume');
    try { await SS.api('/api/pause', { body: { paused: !manual } }); } catch (e) { SS.toast(SS.t('foot.pauseFail')); }
    pollStatus();
  });
  el.setb.addEventListener('click', SS.openSettings);

  document.addEventListener('keydown', (e) => {
    if (e.isComposing || e.defaultPrevented) return;
    /* AltGr (польская, немецкая, чешская… раскладки) приходит как Ctrl+Alt: это набор символа, а не сочетание */
    const altGr = !!(e.getModifierState && e.getModifierState('AltGraph'));
    const ctrl = (e.ctrlKey || e.metaKey) && !altGr;
    const alt = e.altKey && !altGr;
    const code = e.code || '';
    const key = (e.key || '').toLowerCase();
    /* латинская раскладка — по букве на клавише (Dvorak, AZERTY), любая другая (кириллица, греческая…) — по месту клавиши */
    const isKey = (latin, cyr) => (/^[a-z]$/.test(key) ? key === latin : key === cyr || code === 'Key' + latin.toUpperCase());
    const ae = document.activeElement;
    const digit = /^(Digit|Numpad)[1-7]$/.test(code) ? +code.slice(-1) : /^[1-7]$/.test(key) ? +key : 0;
    if (ctrl && !e.shiftKey && !alt && digit) {
      e.preventDefault();
      setTab(R.TABS[digit - 1][0]);
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      if (SS.tauri) hideWindow();
      else clearQuery();
      return;
    }
    if (boot.on) return; /* экран запуска: поле поиска под ним */
    if (isStart()) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (alt) return;
      e.preventDefault();
      move(e.key === 'ArrowDown' ? 1 : -1);
      return;
    }
    if (e.key === 'Enter' && !alt) {
      if (ae && ae.tagName === 'BUTTON' && !ae.hasAttribute('data-i')) return;
      e.preventDefault();
      act(ctrl ? 'reveal' : 'open');
      return;
    }
    if (ctrl && e.shiftKey && !alt && isKey('c', 'с')) { e.preventDefault(); act('copy'); return; }
    if (ctrl && !e.shiftKey && !alt && isKey('f', 'а')) { e.preventDefault(); act('similar'); return; }
    if (!ctrl && !alt && e.key.length === 1 && e.key !== ' ' && ae !== el.q && (!ae || ae.tagName !== 'INPUT')) el.q.focus();
  });

  /* картинка: перетаскивание (подсветка + «дышащая» рамка) и Ctrl+V.
     Подсказку держит не счётчик dragenter/dragleave — WebView2 часто не присылает последний dragleave, когда
     файл уводят из окна или отменяют перенос, и подсказка оставалась висеть. Вместо этого — «пульс» dragover:
     пока файл над окном, он приходит не реже ~350 мс даже без движения мыши; нет его DROP_WATCH_MS — переноса нет. */
  const DROP_WATCH_MS = 700;
  let dropT = 0;
  const hasFiles = (e) => !!(e.dataTransfer && Array.prototype.indexOf.call(e.dataTransfer.types || [], 'Files') >= 0);
  const dropOff = () => { clearTimeout(dropT); el.dropovl.hidden = true; };
  const dropPulse = () => { el.dropovl.hidden = false; clearTimeout(dropT); dropT = setTimeout(dropOff, DROP_WATCH_MS); };
  window.addEventListener('dragenter', (e) => { if (!hasFiles(e)) return; e.preventDefault(); dropPulse(); });
  window.addEventListener('dragover', (e) => { if (!hasFiles(e)) return; e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; dropPulse(); });
  /* ушли за пределы окна (relatedTarget пуст): прячем почти сразу — если следом придёт dragover, значит ещё внутри */
  window.addEventListener('dragleave', (e) => { if (e.relatedTarget) return; clearTimeout(dropT); dropT = setTimeout(dropOff, 80); });
  window.addEventListener('dragend', dropOff);
  window.addEventListener('blur', dropOff);
  document.addEventListener('visibilitychange', () => { if (document.hidden) dropOff(); });
  window.addEventListener('drop', (e) => {
    dropOff();
    if (!hasFiles(e)) return;
    e.preventDefault();
    const files = Array.from(e.dataTransfer.files || []);
    const f = files.find((x) => /^image\//.test(x.type));
    if (f) searchImage(f);
    else if (files.length) SS.toast(SS.t('img.need'));
  });
  document.addEventListener('paste', (e) => {
    const items = e.clipboardData ? Array.from(e.clipboardData.items || []) : [];
    const it = items.find((i) => i.kind === 'file' && /^image\//.test(i.type));
    if (!it) return;
    const f = it.getAsFile();
    if (!f) return;
    e.preventDefault();
    searchImage(f);
  });

  /* окно */
  if (SS.tauri || devCaps) {
    el.caps.hidden = false;
    el.capMin.addEventListener('click', () => SS.invoke('win_minimize').catch(() => {}));
    el.capMax.addEventListener('click', () => SS.invoke('win_toggle_maximize').catch(() => {}));
    el.capHide.addEventListener('click', hideWindow);
    const w = SS.tauri && SS.tauri.window && SS.tauri.window.getCurrentWindow ? SS.tauri.window.getCurrentWindow() : null;
    const maxIcon = () => w && w.isMaximized().then((m) => {
      $('icoMax').hidden = m;
      $('icoRestore').hidden = !m;
      el.capMax.setAttribute('aria-label', SS.t(m ? 'win.restore' : 'win.max'));
    }).catch(() => {});
    if (w) { maxIcon(); w.onResized(maxIcon).catch(() => {}); }
  }
  SS.listen('focus-search', () => {
    if (wasHidden || el.win.classList.contains('w-out') || el.win.classList.contains('w-pre')) animateIn();
    el.q.focus();
    el.q.select();
    pollStatus();
    loadSettings();
  });
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) { clearTimeout(stT); wasHidden = true; return; }
    if (el.win.classList.contains('w-out') || el.win.classList.contains('w-pre')) animateIn();
    pollStatus();
    loadSettings();
  });
  window.addEventListener('focus', () => { if (!document.hidden) loadSettings(); });

  /* смена языка (в настройках или другим окном): статичная разметка уже переведена (i18n.js), остальное — перерисовать */
  SS.onLang(() => {
    el.q.placeholder = S.mode === 'text' ? SS.t('q.ph') : '';
    if (S.sim) setChip(S.sim);
    keysFor = null;
    cache.clear();
    pvCache.clear();
    if (el.tabs.querySelector('.tl')) R.tabs(el.tabs, S.counts, S.tab, '');
    chrome();
    if (!isStart()) schedule(0);
    bootPaint();
    if (SS.tauri) SS.invoke('set_lang', { lang: SS.lang() }).catch(() => {});
  });

  chrome();
  el.q.focus();
  loadSettings();
  pollStatus();
})();

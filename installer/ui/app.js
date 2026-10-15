/* SemSearch Setup — screens and behaviour. Look: design/Installer.dc.html, motion: design/Inst-Motion.dc.html.
   Talks to the core only through the commands and events of docs/INSTALLER.md. Without window.__TAURI__ it loads
   mock.js (a fake core for a normal browser: index.html?mock=<state>). */
(function () {
  'use strict';
  const GiB = window.SSI.GiB;
  const $ = id => document.getElementById(id);

  const NET_STEPS = new Set(['torch', 'libs', 'model', 'vcredist']); // network error here → "no connection" screen
  const SOFT_STEPS = new Set(['mft', 'engine_del']);                  // their failure does not stop the run
  const WEIGHT = { check: 2, vcredist: 3, copy: 3, libs: 4, model: 8, mft: 3, links: 1, close: 2, replace: 4, engine_del: 2, prog_del: 3, data_del: 2 };

  const S = {
    mode: 'install', version: '', lang: 'ru', langPref: 'system', sysLang: 'ru', theme: 'system', sysTheme: null, test: false, installed: null, defaults: {}, gpuInfo: null,
    opts: { root: '', data_dir: '', gpu: false, autostart: true, mft: true, desktop: false },
    disk: null, spaceErr: false,
    scr: null,
    plan: [], steps: {}, softErr: {}, failed: null, ready: false, running: false, cur: null,
    doneInfo: null, launch: true, wipe: false, mftRow: 'ok', engFailed: false
  };
  let api = null;
  let t = window.SSI.ru;
  let V = null;        // the mounted screen: its element refs
  let quiet = false;   // true while re-rendering in place (language switch, new disk numbers): no entrance motion
  let M = {};          // what the previous screen showed, for the entrance (the mark stays put between hero screens)

  /* ---------- DOM helpers ---------- */
  function h(tag, a, ...kids) {
    const el = document.createElement(tag);
    if (a) for (const k in a) {
      const v = a[k];
      if (v == null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = v;
      else if (k.slice(0, 2) === 'on') el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) el.append(c);
    return el;
  }
  function svg(markup) {
    const tp = document.createElement('template');
    tp.innerHTML = markup.trim();
    return tp.content.firstElementChild;
  }
  const A = cls => (quiet ? '' : cls);
  function replay(el, cls) { if (!el) return; el.classList.remove(cls); void el.getBoundingClientRect(); el.classList.add(cls); }

  const ICON = {
    folder: 'M3 6.5h6l2 2h10V19H3z',
    chip: 'M7 7h10v10H7z M10 3v4 M14 3v4 M10 17v4 M14 17v4 M3 10h4 M3 14h4 M17 10h4 M17 14h4',
    file: 'M6 3h8l4 4v14H6z M14 3v4h4 M9 12h6 M9 16h4',
    disk: 'M3 14h18v5H3z M3 14l2.6-8h12.8L21 14 M7 16.5h.01',
    tray: 'M3 5h18v14H3z M3 15h18 M14 17h.01 M17.5 17h.01',
    shield: 'M12 3l7 3v5.5c0 4.3-2.9 7.9-7 9.5c-4.1-1.6-7-5.2-7-9.5V6z',
    warn: 'M12 9v4 M12 16.5h.01 M10.3 3.6L2.4 17.5A2 2 0 0 0 4.1 20.5h15.8a2 2 0 0 0 1.7-3L13.7 3.6a2 2 0 0 0-3.4 0z',
    sync: 'M20 12a8 8 0 1 1-2.4-5.7 M20 4v4.5h-4.5'
  };
  const icon18 = n => svg(`<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${ICON[n]}"></path></svg>`);
  const check12 = () => svg('<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12l5 5L20 7"></path></svg>');

  function stepIcon(st) {
    if (st === 'done') return svg(`<svg class="c-acc" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path class="${A('chk')}" d="M5 12l5 5L20 7"></path></svg>`);
    if (st === 'active') return svg('<svg class="spin" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke-width="2.6" stroke-linecap="round" aria-hidden="true"><circle class="sp-track" cx="12" cy="12" r="9"></circle><path class="sp-arc" d="M12 3a9 9 0 0 1 9 9"></path></svg>');
    if (st === 'failed') return svg(`<svg class="c-err ${A('pop')}" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"></path></svg>`);
    return h('span', { class: 'wdot' });
  }
  function markSvg(kind, dot) {
    if (kind === 'warn') return svg(`<svg class="wn ${A('shake')}" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${ICON.warn}"></path></svg>`);
    if (kind === 'check') return svg(`<svg class="ok ${A('pop')}" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18z"></path><path class="${A('chk')}" d="M8 12.2l2.8 2.8L16.2 9.5"></path></svg>`);
    return svg(`<svg class="lg" width="56" height="56" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d="M28.79 106.54a79.25 79.25 0 1 1 158.5 0a79.25 79.25 0 1 1 -158.5 0ZM59.29 106.54a48.75 48.75 0 1 0 97.5 0a48.75 48.75 0 1 0 -97.5 0Z"></path><g class="${dot || ''}"><path d="M172.71 198.46a27.25 27.25 0 1 1 54.5 0a27.25 27.25 0 1 1 -54.5 0Z"></path></g></svg>`);
  }

  /* info / step row: icon, label (+ note), right text, action button */
  function row(o) {
    const ic = h('span', { class: 'ic' + (o.ic ? ' ' + o.ic : '') }, o.iconEl || icon18(o.icon));
    const lb = h('span', { class: 'lb' + (o.ls ? ' ' + o.ls : ''), text: o.label });
    const tx = h('span', { class: 'tx' }, lb, o.note ? h('span', { class: 'nt' + (o.sel ? ' sel' : ''), text: o.note }) : null);
    const rt = h('span', { class: 'rt' + (o.rc ? ' ' + o.rc : ''), text: o.right || '' });
    rt.hidden = !o.right;
    const el = h('div', { class: 'row' + (o.step ? ' st' : '') }, ic, tx, rt,
      o.act ? h('button', { class: 'btn2 sm', text: o.act.label, onclick: o.act.on }) : null);
    return { el, ic, lb, rt };
  }

  /* the centred screen: mark, title, subtitle, optional bar, card of rows */
  function hero(o) {
    const mark = h('span', { class: 'mark' + (!quiet && !M.hero ? ' grow' : '') }, markSvg(o.mark, o.dot));
    const title = h('h1', { text: o.title || '' });
    const sub = h('p', { class: 'sub sel', text: o.sub || '' });
    let bar = null, fill = null;
    if (o.bar) {
      fill = h('i');
      bar = h('div', { class: 'pbar', role: 'progressbar', 'aria-label': o.title || '', 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': '0' }, fill);
    }
    let card = null;
    if (o.card || (o.rows && o.rows.length)) {
      card = h('div', { class: 'card' });
      (o.rows || []).forEach((r, i) => {
        if (!quiet) { r.classList.add('rs'); r.style.animationDelay = (160 + i * 60) + 'ms'; }
        card.append(r);
      });
    }
    const el = h('div', { class: 'hero' }, h('div', { class: 'col' }, mark, title, sub, bar, card));
    return { el, mark, title, sub, bar, fill, card, isHero: true, markKind: o.mark || 'logo', dot: o.dot || '' };
  }

  /* ---------- formatting / state helpers ---------- */
  const drv = d => String(d || '').replace(/[\\/]+$/, '');
  const errText = e => (typeof e === 'string' ? e : (e && (e.message || e.error)) || String(e || ''));
  const gpuOk = () => !!(S.gpuInfo && S.gpuInfo.supported);
  const needBytes = () => (S.disk && S.disk.need_bytes) || (S.opts.gpu ? 7 : 3.5) * GiB;
  const isLow = () => S.spaceErr || !!(S.disk && S.disk.ok === false);
  function shortage() {
    const d = S.disk || {}, need = needBytes();
    if (d.data_drive && d.data_free_bytes != null && (d.free_bytes == null || d.free_bytes >= need) && d.data_free_bytes < 2 * GiB)
      return { drive: drv(d.data_drive), need: 2 * GiB, free: d.data_free_bytes };
    return { drive: drv(d.drive), need, free: d.free_bytes || 0 };
  }
  function gpuNote() {
    const g = S.gpuInfo;
    if (!g) return t.gpuNone;
    if (g.supported) return t.gpuNote(g.name || 'NVIDIA');
    const r = String(g.reason || '');
    if (/driver|драйвер/i.test(r)) return t.gpuDriver(g.name || 'NVIDIA');
    if (!r || /^(cap|old|compute)/i.test(r)) return t.gpuOld(g.name || 'NVIDIA');
    return (g.name ? g.name + ' · ' : '') + r;
  }
  /* effective options of the run: chosen ones on install, install.json on update/uninstall */
  function cfg() {
    if (S.mode === 'install') return S.opts;
    const i = S.installed || {}, d = S.defaults || {};
    return { root: i.root || d.root || '', data_dir: i.data_dir || d.data_dir || '', gpu: !!i.gpu, autostart: !!i.autostart, mft: !!i.mft, desktop: !!i.desktop };
  }
  /* a picked folder gets its own SemSearch subfolder (uninstall removes <root> and, on request, D entirely) */
  function withAppDir(p) {
    const s = String(p).replace(/[\\/]+$/, '');
    const last = s.split(/[\\/]/).pop();
    return /semsearch/i.test(last) ? s : s + '\\SemSearch';
  }
  /* command arguments: snake_case as in the contract, plus camelCase twins (Tauri's default for command args) */
  function args(o) {
    const r = Object.assign({}, o);
    for (const k in o) if (k.includes('_')) r[k.replace(/_([a-z])/g, (m, c) => c.toUpperCase())] = o[k];
    return r;
  }
  const home = () => (S.mode === 'install' ? 'welcome' : S.mode);

  function stepLabel(id) {
    const c = cfg(), upd = S.mode === 'update';
    switch (id) {
      case 'check': return t.sCheck;
      case 'vcredist': return t.sVc;
      case 'copy': return t.sCopy;
      case 'torch': return c.gpu ? t.sTorchGpu : t.sTorchCpu;
      case 'libs': return upd ? t.sLibsUp : t.sLibs;
      case 'model': return t.sModel;
      case 'mft': return upd ? t.sEngUp : t.sMft;
      case 'links': return c.autostart ? t.sLinksAuto : t.sLinks;
      case 'close': return t.sClose;
      case 'replace': return t.sReplace;
      case 'engine_del': return t.sEngDel;
      case 'prog_del': return t.sProgDel;
      case 'data_del': return t.sDataDel;
    }
    return id;
  }
  const weight = id => (id === 'torch' ? (cfg().gpu ? 14 : 4) : WEIGHT[id] || 2);
  function waitText(id) {
    if (id === 'mft' || id === 'engine_del') return t.sWaitWin;
    if (id === 'vcredist') return t.sVcWait;
    return '';
  }
  function guessPlan() {
    const c = cfg();
    if (S.mode === 'update') return ['close', 'replace', 'libs'].concat(c.mft ? ['mft'] : [], ['links']);
    if (S.mode === 'uninstall') return ['close', 'engine_del', 'prog_del'].concat(S.wipe ? ['data_del'] : []);
    return ['check', 'copy', 'torch', 'libs', 'model'].concat(c.mft ? ['mft'] : [], ['links']);
  }

  /* ---------- footer ---------- */
  const B = (label, kind, on, off) => ({ label, kind, on, off: !!off });
  function setFoot(f) {
    f = f || {};
    if (!f.lang) closeDropdowns();
    $('ddLang').hidden = !f.lang;
    $('ddTheme').hidden = !f.lang;
    $('launchBtn').hidden = !f.launch;
    $('launchBtn').setAttribute('aria-checked', String(S.launch));
    const n = $('fnote');
    n.hidden = !f.note;
    if (f.note) { n.textContent = f.note.text; n.classList.toggle('err', !!f.note.err); }
    $('flog').hidden = !f.log;
    const box = $('fbtns'), btns = f.btns || [];
    btns.forEach((b, i) => {
      let el = box.children[i];
      if (!el) {
        el = h('button', {});
        el.addEventListener('click', () => { if (el._on) el._on(); });
        box.append(el);
      }
      el.textContent = b.label;
      el.className = 'fb ' + (b.kind === 'sec' ? 'sec btn2' : (b.kind === 'dan' ? 'dan' : '') + (b.off ? '' : ' prim'));
      el.disabled = b.off;
      el._on = b.off ? null : b.on;
      el.dataset.kind = b.kind;
    });
    while (box.children.length > btns.length) box.lastElementChild.remove();
  }
  function texts() {
    document.documentElement.lang = S.lang;
    document.title = t.bar[S.mode] || t.bar.install;
    $('ttl').textContent = t.bar[S.mode] || t.bar.install;
    $('capMin').setAttribute('aria-label', t.min);
    $('capClose').setAttribute('aria-label', t.close);
    renderDropdowns();
    $('launchTxt').textContent = t.launch;
    $('flog').textContent = t.log;
  }

  /* ---------- screens ---------- */
  const SCR = {};

  SCR.welcome = function () {
    const o = S.opts, low = isLow();
    let disk;
    if (low) {
      const s = shortage();
      disk = row({ icon: 'disk', ic: 'err', ls: 's-err', label: t.lowLabel(s.drive), note: t.lowNote(t.size(s.need), t.size(s.free)), act: { label: t.otherFolder, on: () => go('options') } });
    } else {
      disk = row({ icon: 'disk', label: t.space(t.size(needBytes())), note: S.disk ? t.free(t.size(S.disk.free_bytes), drv(S.disk.drive)) : '' });
    }
    const v = hero({
      mark: 'logo', title: 'SemSearch', sub: t.welcomeSub,
      rows: [
        row({ icon: 'folder', label: t.progFolder, note: o.root, sel: true }).el,
        row({ icon: 'chip', label: o.gpu ? t.onGpu : t.onCpu, note: o.gpu ? ((S.gpuInfo && S.gpuInfo.name) || '') : t.cpuShort }).el,
        row({ icon: 'file', label: o.mft ? t.mftOn : t.mftOff, note: o.mft ? t.mftOnNote : t.mftOffNote }).el,
        disk.el
      ]
    });
    v.foot = { lang: true, btns: [B(t.settings, 'sec', () => go('options')), B(t.install, 'pri', startInstall, low)] };
    return v;
  };

  function footOptions() {
    const low = isLow(), d = S.disk;
    let note;
    if (low) {
      const s = shortage();
      note = { text: t.lowFoot(s.drive, t.size(s.need), t.size(s.free)), err: true };
    } else {
      note = { text: t.space(t.size(needBytes())) + (d ? ' · ' + t.freeShort(t.size(d.free_bytes), drv(d.drive)) : '') };
    }
    return { note, btns: [B(t.back, 'sec', () => go('welcome')), B(t.install, 'pri', startInstall, low)] };
  }

  SCR.options = function () {
    const o = S.opts;
    const progPath = h('div', { class: 'nt path sel', text: o.root });
    const dataPath = h('div', { class: 'nt path sel', text: o.data_dir });
    const folder = (label, extra, pathEl, key) => h('div', { class: 'frow' },
      h('div', { class: 'tx' }, h('div', {}, label, extra), pathEl),
      h('button', { class: 'btn2', text: t.change, onclick: () => changeFolder(key, pathEl) }));

    const radio = (label, note, on, disabled, pick) => h('button', { class: 'opt', role: 'radio', 'aria-checked': String(on), disabled, onclick: pick },
      h('span', { class: 'dot' }),
      h('span', { class: 'tx' }, h('span', { text: label }), h('span', { class: 'nt', text: note })));
    const rGpu = radio(t.gpu, gpuNote(), o.gpu, !gpuOk(), () => pickGpu(true));
    const rCpu = radio(t.cpu, t.cpuNote, !o.gpu, false, () => pickGpu(false));
    const group = h('div', { class: 'card', role: 'radiogroup', 'aria-labelledby': 'o-model' }, rGpu, rCpu);
    group.addEventListener('keydown', e => {
      if (!/^Arrow(Up|Down|Left|Right)$/.test(e.key)) return;
      e.preventDefault();
      const next = document.activeElement === rGpu ? rCpu : rGpu;
      if (next.disabled) return;
      next.focus();
      next.click();
    });

    const sw = (id, label, note, key) => {
      const b = h('button', { class: 'sw', role: 'switch', 'aria-checked': String(o[key]), 'aria-labelledby': id }, h('span', { class: 'knob' }));
      b.addEventListener('click', () => { o[key] = !o[key]; b.setAttribute('aria-checked', String(o[key])); });
      return h('div', { class: 'frow sp' }, h('div', { class: 'tx' }, h('div', { id, text: label }), h('div', { class: 'dn', text: note })), b);
    };

    const el = h('div', { class: 'opts' },
      h('h1', { text: t.optTitle }),
      h('section', { 'aria-labelledby': 'o-where' },
        h('h2', { id: 'o-where', text: t.where }),
        h('div', { class: 'card' },
          folder(t.progFolder, null, progPath, 'root'),
          folder(t.dataFolder, [' ', h('span', { class: 'dn', text: '· ' + t.dataNote })], dataPath, 'data_dir'))),
      h('section', { 'aria-labelledby': 'o-model' },
        h('h2', { id: 'o-model', text: t.computeOn }), group),
      h('section', { 'aria-labelledby': 'o-start' },
        h('h2', { id: 'o-start', text: t.startSec }),
        h('div', { class: 'card' },
          sw('o-auto', t.auto, t.autoNote, 'autostart'),
          sw('o-mft', t.mft, t.mftNote, 'mft'),
          sw('o-desk', t.desk, t.deskNote, 'desktop'))));

    function pickGpu(on) {
      if (S.opts.gpu === on) return;
      S.opts.gpu = on;
      rGpu.setAttribute('aria-checked', String(on));
      rCpu.setAttribute('aria-checked', String(!on));
      refreshDisk(true);
    }
    return { el, foot: footOptions(), esc: () => go('welcome') };
  };

  async function changeFolder(key, pathEl) {
    let p = null;
    try { p = await api.invoke('pick_folder', { initial: S.opts[key] }); } catch (e) { return; }
    if (!p) return;
    S.opts[key] = withAppDir(p);
    pathEl.textContent = S.opts[key];
    refreshDisk(true);
  }

  /* progress — also the "no connection" and the generic error screen (failed state) */
  SCR.progress = function () {
    const v = hero({ mark: 'logo', dot: 'orbit', title: '', sub: '', bar: true, card: true });
    v.rows = new Map();
    v.built = false;
    v.after = () => { updProgress(); v.built = true; };
    return v;
  };

  function syncRows() {
    const card = V.card, cur = V.rows;
    for (const [id, r] of cur) if (!S.plan.includes(id)) { r.el.remove(); cur.delete(id); }
    S.plan.forEach((id, i) => {
      let r = cur.get(id);
      if (!r) {
        r = row({ iconEl: stepIcon('wait'), label: stepLabel(id), ls: 's-wait', step: true });
        r.st = 'wait';
        if (!quiet) { r.el.classList.add('rs'); r.el.style.animationDelay = (V.built ? 0 : 160 + i * 60) + 'ms'; }
        cur.set(id, r);
      }
      const at = card.children[i];
      if (at !== r.el) card.insertBefore(r.el, at || null);
    });
  }

  function progressPct() {
    if (S.ready) return 100;
    let tot = 0, got = 0;
    for (const id of S.plan) {
      const w = weight(id), s = S.steps[id] || {};
      const fatal = S.failed && S.failed.id === id;
      tot += w;
      if (s.state === 'done' || s.state === 'skipped' || (s.state === 'failed' && !fatal)) got += w;
      else if ((s.state === 'active' || fatal) && s.total > 0 && s.got > 0) got += w * Math.min(1, s.got / s.total);
    }
    return tot ? Math.min(100, Math.round(got / tot * 100)) : 0;
  }

  function failText(f) {
    const s = S.steps[f.id] || {};
    if (f.kind === 'network' && NET_STEPS.has(f.id)) {
      if (f.id === 'vcredist') return { title: t.netTitle.vcredist, sub: t.netSubRetry };
      return { title: t.netTitle[f.id], sub: s.got > 0 ? t.netSub(t.of(s.got, s.total)) : t.netSubPlain };
    }
    if (f.kind === 'uac_denied') return f.id === 'vcredist' ? { title: t.vcTitle, sub: t.vcUacSub } : { title: t.errTitle[S.mode], sub: t.uacSub };
    return { title: t.errTitle[S.mode], sub: f.message || t.errSubLog };
  }

  function updProgress() {
    if (S.scr !== 'progress' || !V || !V.rows) return;
    syncRows();
    const f = S.failed;
    const want = f ? 'warn' : 'logo', dot = S.ready ? 'settle' : 'orbit';
    if (V.markKind !== want) {
      V.mark.replaceChildren(markSvg(want, dot));
      V.markKind = want;
      V.dot = want === 'logo' ? dot : '';
    } else if (want === 'logo' && V.dot !== dot) {
      V.mark.querySelector('g').setAttribute('class', dot);
      V.dot = dot;
    }
    let title, sub;
    if (f) ({ title, sub } = failText(f));
    else {
      title = S.mode === 'update' ? t.updating : S.mode === 'uninstall' ? t.removing : t.progTitle;
      sub = S.ready ? t.readyNow : stepLabel(S.cur || S.plan[0]) + '…';
    }
    V.title.textContent = title;
    V.sub.textContent = sub;
    V.sub.title = f ? sub : '';
    const pct = progressPct();
    V.bar.setAttribute('aria-label', title);
    V.bar.setAttribute('aria-valuenow', String(pct));
    V.bar.classList.toggle('err', !!f);
    V.fill.style.width = pct + '%';

    for (const id of S.plan) {
      const r = V.rows.get(id), s = S.steps[id] || {};
      const fatal = f && f.id === id;
      const st = fatal ? 'failed' : (s.state || 'wait');
      if (r.st !== st) {
        r.st = st;
        r.ic.replaceChildren(stepIcon(st));
        r.lb.className = 'lb ' + ({ failed: 's-err', active: 's-act', done: 's-done', skipped: 's-done' }[st] || 's-wait');
      }
      r.lb.textContent = stepLabel(id);
      let right = '';
      if (st === 'failed') right = fatal ? (t.errRight[f.kind] || t.errRight.other) : (S.softErr[id] ? t.errRight[S.softErr[id]] || '' : '');
      else if (st === 'active') right = (s.got != null && (s.total || s.got)) ? t.of(s.got, s.total) : (waitText(id) || s.note || '');
      else if (st === 'done') right = id === 'check' ? t.sCheckOk : '';
      else if (st === 'skipped') right = t.sSkipped;
      r.rt.textContent = right;
      r.rt.hidden = !right;
      r.rt.classList.toggle('err', st === 'failed');
    }

    if (f) setFoot({ log: true, btns: [B(t.cancel, 'sec', cancelRun), B(t.retry, 'pri', retryRun)] });
    else if (S.mode === 'install') setFoot({ note: { text: t.canMin }, btns: [B(t.cancel, 'sec', cancelRun)] });
    else if (S.mode === 'update') setFoot({ note: { text: t.updNote } });
    else setFoot({});
  }

  function mftRow() {
    const upd = S.mode === 'update';
    if (S.mftRow === 'active') return row({ iconEl: stepIcon('active'), label: upd ? t.sEngUp : t.sMft, right: t.sWaitWin }).el;
    if (S.mftRow === 'failed') return row({ icon: 'warn', ic: 'warn', label: upd ? t.engNotUp : t.noAdm, note: upd ? t.engNotUpNote : t.noAdmNote, act: { label: upd ? t.retry : t.connect, on: retryMft } }).el;
    return row({ icon: 'file', label: t.byName, note: t.byNameNote }).el;
  }

  SCR.done = function () {
    const upd = S.mode === 'update', rows = [];
    let mft = null;
    if (!upd) {
      mft = mftRow();
      rows.push(row({ icon: 'chip', label: t.firstRun, note: t.firstRunNote }).el, mft,
        row({ icon: 'tray', label: t.tray, note: cfg().autostart ? t.trayAuto : t.trayNoAuto }).el);
    } else if (S.mftRow !== 'ok') {
      mft = mftRow();
      rows.push(mft);
    }
    // coming from the finished progress screen the dot has already settled
    const v = hero({ mark: 'logo', dot: M.dot === 'settle' ? '' : A('settle'), title: upd ? t.updated : t.doneTitle, sub: upd ? t.updatedSub(S.version) : t.doneSub, rows });
    v.mftEl = mft;
    v.foot = { launch: true, btns: [B(t.finish, 'pri', () => finishNow(S.launch))] };
    return v;
  };

  function updDoneMft() {
    if (S.scr !== 'done' || !V) return;
    if (!V.mftEl || (S.mode === 'update' && S.mftRow === 'ok')) { mount(false); return; }
    const el = mftRow();
    V.mftEl.replaceWith(el);
    V.mftEl = el;
  }

  SCR.update = function () {
    const ins = S.installed || {};
    const rows = [row({ icon: 'disk', label: t.keep, note: t.keepNote }).el, row({ icon: 'sync', label: t.closes, note: t.closesNote }).el];
    if (ins.mft) rows.push(row({ icon: 'shield', label: t.eng, note: t.engNote }).el);
    const v = hero({ mark: 'logo', title: t.updTitle, sub: t.updSub(ins.version || '—', S.version), rows });
    v.foot = { btns: [B(t.notNow, 'sec', () => finishNow(false)), B(t.update, 'pri', startUpdate)] };
    v.esc = () => finishNow(false);
    return v;
  };

  function boxRow(label, note, checked, disabled, onToggle) {
    const b = h('button', { class: 'opt', role: 'checkbox', 'aria-checked': String(checked), disabled },
      h('span', { class: 'box' }, check12()),
      h('span', { class: 'tx' }, h('span', { class: 'lb13', text: label }), h('span', { class: 'nt', text: note })));
    if (onToggle) b.addEventListener('click', () => {
      const on = b.getAttribute('aria-checked') !== 'true';
      b.setAttribute('aria-checked', String(on));
      if (on) replay(b.querySelector('path'), 'chk');
      onToggle(on);
    });
    return b;
  }

  SCR.uninstall = function () {
    const v = hero({
      mark: 'logo', title: t.unTitle, sub: t.unSub,
      rows: [boxRow(t.unProg, cfg().root, true, true, null), boxRow(t.unData, t.unDataNote, S.wipe, false, on => { S.wipe = on; })]
    });
    v.foot = { btns: [B(t.cancel, 'sec', () => finishNow(false)), B(t.remove, 'dan', startUninstall)] };
    v.esc = () => finishNow(false);
    return v;
  };

  SCR.removed = function () {
    const di = S.doneInfo || {}, data = di.data_dir || cfg().data_dir, rows = [];
    if (!S.wipe) rows.push(row({ icon: 'folder', label: t.dataFolder, note: data, sel: true, act: { label: t.openFolder, on: () => api.invoke('open_path', { path: data }).catch(() => {}) } }).el);
    if (S.engFailed) rows.push(row({ icon: 'warn', ic: 'warn', label: t.engNotDel, note: t.engNotDelNote }).el);
    const v = hero({ mark: 'check', title: t.removed, sub: S.wipe ? t.removedAll : t.removedKeep, rows });
    v.foot = { btns: [B(t.closeBtn, 'pri', () => finishNow(false))] };
    return v;
  };

  /* get_state failed: nothing else to show */
  SCR.fatal = function () {
    const v = hero({ mark: 'warn', title: t.errTitle.install, sub: S.fatal || t.errSubLog });
    v.foot = { log: true, btns: [B(t.closeBtn, 'pri', () => closeWindow())] };
    return v;
  };

  /* ---------- navigation ---------- */
  // ---------- язык и тема: выпадающие списки, как в настройках программы (открываются вверх)
  // «Как в Windows» — язык и тема Windows; в настройки программы записывается только выбранное руками.
  const DD = {
    lang: () => ({ el: $('ddLang'), cur: S.langPref, aria: t.langAria,
      opts: [['ru', 'Русский'], ['en', 'English'], ['system', t.sysLang]],
      pick: v => { S.langPref = v; S.lang = v === 'system' ? S.sysLang : v; t = window.SSI[S.lang]; mount(false); } }),
    theme: () => ({ el: $('ddTheme'), cur: S.theme, aria: t.themeAria,
      opts: [['light', t.themeNames.light], ['dark', t.themeNames.dark], ['system', t.themeNames.system]],
      pick: v => { S.theme = v; applyTheme(); mount(false); } })
  };
  function renderDropdowns() {
    for (const k of Object.keys(DD)) {
      const d = DD[k]();
      d.el.querySelector('.ddb').setAttribute('aria-label', d.aria);
      d.el.querySelector('.ddl').textContent = d.opts.find(o => o[0] === d.cur)[1];
      // у SVG нет свойства hidden — только атрибут
      for (const s of d.el.querySelectorAll('svg[data-th]')) s.toggleAttribute('hidden', s.dataset.th !== S.theme);
    }
  }
  function closeDropdowns(focusKey) {
    for (const k of Object.keys(DD)) {
      const el = DD[k]().el, p = el.querySelector('.ddp');
      if (p.hidden) continue;
      p.hidden = true;
      p.textContent = '';
      el.classList.remove('open');
      el.querySelector('.ddb').setAttribute('aria-expanded', 'false');
      if (focusKey === k) el.querySelector('.ddb').focus();
    }
  }
  function openDropdown(k) {
    closeDropdowns();
    const d = DD[k](), p = d.el.querySelector('.ddp');
    p.setAttribute('aria-label', d.aria);
    for (const [v, label] of d.opts) {
      const o = h('button', { class: 'ddo', role: 'option', 'aria-selected': String(v === d.cur), text: label });
      o.addEventListener('click', () => { closeDropdowns(k); if (v !== d.cur) d.pick(v); });
      p.appendChild(o);
    }
    p.hidden = false;
    d.el.classList.add('open');
    d.el.querySelector('.ddb').setAttribute('aria-expanded', 'true');
    (p.querySelector('[aria-selected=true]') || p.firstChild).focus();
  }

  function applyTheme() {
    const html = document.documentElement, want = S.theme === 'system' ? S.sysTheme : S.theme;
    html.classList.remove('dark', 'light');
    if (want) html.classList.add(want);
  }

  function mount(anim) {
    quiet = !anim;
    M = { hero: !!(V && V.isHero), dot: V && V.dot };
    const v = SCR[S.scr]();
    const view = h('div', { class: 'view' + (anim ? ' in' : ''), id: 'view' }, v.el);
    $('view').replaceWith(view);
    V = v;
    // a footer button clicked with the mouse keeps focus across screens; drop it so Enter goes to the primary button
    const fa = document.activeElement;
    if (fa && fa !== document.body && !fa.matches(':focus-visible')) fa.blur();
    texts();
    if (v.foot) setFoot(v.foot);
    if (v.after) v.after();
    quiet = false;
  }
  function go(scr) { S.scr = scr; mount(true); }

  let diskSeq = 0;
  async function refreshDisk(changed, silent) {
    if (changed) S.spaceErr = false;
    const my = ++diskSeq, o = S.opts;
    let d = null;
    try { d = await api.invoke('disk_info', args({ root: o.root, data_dir: o.data_dir, gpu: o.gpu })); } catch (e) { d = null; }
    if (my !== diskSeq) return;
    S.disk = d;
    if (silent) return;
    if (S.scr === 'options') setFoot(footOptions());
    else if (S.scr === 'welcome') mount(false);
  }

  /* ---------- the run ---------- */
  function startRun(params) {
    Object.assign(S, { plan: guessPlan(), steps: {}, softErr: {}, failed: null, ready: false, running: true, cur: null, doneInfo: null });
    go('progress');
    const p = Object.assign({ root: '', data_dir: '', gpu: false, autostart: false, mft: false, desktop: false, wipe: false }, params, { lang: S.langPref, theme: S.theme });
    api.invoke('start', args(p)).catch(e => {
      if (S.running && !S.failed) { S.failed = { id: S.cur || S.plan[0], kind: 'other', message: errText(e) }; updProgress(); }
    });
  }
  function startInstall() { if (!isLow()) startRun(Object.assign({}, S.opts)); }
  function startUpdate() { startRun(cfg()); }
  function startUninstall() { startRun(Object.assign({}, cfg(), { wipe: S.wipe })); }

  function retryRun() {
    const f = S.failed;
    if (!f) return;
    S.failed = null;
    if (S.steps[f.id]) S.steps[f.id].state = 'active';
    S.cur = f.id;
    updProgress();
    api.invoke('retry').catch(e => {
      if (S.running && !S.failed) { S.failed = { id: f.id, kind: 'other', message: errText(e) }; updProgress(); }
    });
  }
  function cancelRun() {
    S.running = false;
    S.failed = null;
    api.invoke('cancel').catch(() => {});
    go(home());
    if (S.mode === 'install') refreshDisk(false);
  }
  function retryMft() {
    S.mftRow = 'active';
    updDoneMft();
    api.invoke('retry_mft').catch(() => { S.mftRow = 'failed'; updDoneMft(); });
  }

  let leaving = false;
  function finishNow(launch) {
    if (leaving) return;
    leaving = true;
    $('iw').classList.add('leave');
    setTimeout(() => {
      api.invoke('finish', { launch: !!launch }).catch(() => closeWindow());
    }, 120);
  }
  async function closeWindow() {
    try { await api.win().close(); } catch (e) { /* nothing else to do */ }
  }

  /* ---------- core events ---------- */
  function onPlan(p) {
    if (!S.running || !p || !Array.isArray(p.steps)) return;
    S.plan = p.steps.slice();
    updProgress();
  }
  function onStep(p) {
    if (!p || !p.id) return;
    if (!S.running) {
      if (p.id === 'mft' && S.scr === 'done') {   // retry_mft() after setup
        S.mftRow = p.state === 'active' ? 'active' : p.state === 'done' ? 'ok' : 'failed';
        updDoneMft();
      }
      return;
    }
    S.steps[p.id] = Object.assign({}, S.steps[p.id], p);
    if (p.state === 'active') S.cur = p.id;
    if (!S.plan.includes(p.id)) S.plan.push(p.id);
    updProgress();
  }
  function onError(p) {
    if (!S.running || !p || p.kind === 'cancelled') return;
    if (SOFT_STEPS.has(p.id)) { S.softErr[p.id] = p.kind; updProgress(); return; }
    if (p.kind === 'space' && S.mode === 'install') {
      // "not enough space": stop the run and show the welcome screen with the low-space row
      S.running = false;
      api.invoke('cancel').catch(() => {});
      S.spaceErr = true;
      go('welcome');
      refreshDisk(false);
      return;
    }
    S.failed = p;
    updProgress();
  }
  function onDone(p) {
    if (!p || !S.running) return;
    S.doneInfo = p;
    if (p.ok === false) {
      if (!S.failed) { S.failed = { id: S.cur || S.plan[0], kind: 'other', message: p.message || '' }; updProgress(); }
      return;
    }
    S.running = false;
    S.ready = true;
    S.mftRow = cfg().mft && p.mft_ok === false ? 'failed' : 'ok';
    S.engFailed = (S.steps.engine_del || {}).state === 'failed';
    updProgress();
    setTimeout(() => go(S.mode === 'uninstall' ? 'removed' : 'done'), 600);
  }

  /* ---------- window chrome, keyboard ---------- */
  function wire() {
    $('capMin').addEventListener('click', () => { try { api.win().minimize(); } catch (e) { /* ignore */ } });
    $('capClose').addEventListener('click', async () => {
      if (S.scr === 'done' || S.scr === 'removed') return finishNow(false);
      if (S.running) {
        S.running = false;
        try { await api.invoke('cancel'); } catch (e) { /* closing anyway */ }
      }
      closeWindow();
    });
    for (const k of Object.keys(DD)) {
      DD[k]().el.querySelector('.ddb').addEventListener('click', () => {
        if (DD[k]().el.classList.contains('open')) closeDropdowns(k); else openDropdown(k);
      });
    }
    document.addEventListener('pointerdown', e => { if (!e.target.closest('.dd')) closeDropdowns(); });
    $('launchBtn').addEventListener('click', () => {
      S.launch = !S.launch;
      $('launchBtn').setAttribute('aria-checked', String(S.launch));
      if (S.launch) replay($('launchBtn').querySelector('path'), 'chk');
    });
    $('flog').addEventListener('click', e => { e.preventDefault(); api.invoke('open_log').catch(() => {}); });

    document.addEventListener('keydown', e => {
      const openDd = document.querySelector('.dd.open');
      if (openDd) {  // открытый список: Esc/Tab закрывают, стрелки ходят по пунктам, Enter — нажатие пункта
        const k = openDd.id === 'ddLang' ? 'lang' : 'theme';
        if (e.key === 'Escape' || e.key === 'Tab') {
          if (e.key === 'Escape') e.preventDefault();
          closeDropdowns(e.key === 'Escape' ? k : null);
          return;
        }
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          const os = [...openDd.querySelectorAll('.ddo')], i = os.indexOf(document.activeElement);
          os[(i + (e.key === 'ArrowDown' ? 1 : os.length - 1)) % os.length].focus();
          return;
        }
      }
      if (e.key === 'Enter' && !e.repeat && !e.altKey && !e.ctrlKey) {
        const a = document.activeElement;
        // a focused push button or link takes Enter itself; switches, radios and checkboxes don't (as in Windows dialogs)
        if (a && a !== document.body && a.closest('button, a') && !a.closest('[role=switch], [role=radio], [role=checkbox]')) return;
        const b = [...$('fbtns').children].reverse().find(x => x.dataset.kind !== 'sec');
        if (!b || b.disabled) return;
        e.preventDefault();
        b.classList.add('kpress');
        setTimeout(() => { b.classList.remove('kpress'); b.click(); }, 100);
      } else if (e.key === 'Escape' && V && V.esc && !leaving) {
        e.preventDefault();
        V.esc();
      }
    });
    if (api.real) {
      // a page reload or the browser menu would lose a running setup
      document.addEventListener('contextmenu', e => { if (!e.target.closest('.sel')) e.preventDefault(); });
      document.addEventListener('keydown', e => {
        if (e.key === 'F5' || e.key === 'F3' || e.key === 'F7' || (e.ctrlKey && /^[rpfgjuo]$/i.test(e.key)) || (e.altKey && /^Arrow(Left|Right)$/.test(e.key))) e.preventDefault();
      }, true);
    }
  }

  function makeApi() {
    const T = window.__TAURI__;
    if (T && T.core) {
      return {
        real: true,
        invoke: (cmd, a) => T.core.invoke(cmd, a || {}),
        listen: (name, cb) => T.event.listen(name, e => cb(e.payload)),
        win: () => T.window.getCurrentWindow()
      };
    }
    return window.SSMock;
  }
  function loadScript(src) {
    return new Promise((res, rej) => {
      const s = document.createElement('script');
      s.src = src;
      s.onload = res;
      s.onerror = rej;
      document.head.append(s);
    });
  }

  async function boot() {
    if (!(window.__TAURI__ && window.__TAURI__.core)) await loadScript('mock.js');
    api = makeApi();
    wire();
    let st = null;
    try { st = await api.invoke('get_state'); } catch (e) { S.fatal = errText(e); }
    if (!st) { S.scr = 'fatal'; mount(true); return; }
    S.mode = ['install', 'update', 'uninstall'].includes(st.mode) ? st.mode : 'install';
    S.version = st.version || '';
    S.sysLang = st.lang === 'en' ? 'en' : 'ru';  // язык Windows (не русский — английский); по умолчанию — «как в Windows»
    S.lang = S.sysLang;
    const html = document.documentElement;  // mock.js может навязать тему Windows классом — она и есть «как в Windows»
    S.sysTheme = html.classList.contains('dark') ? 'dark' : html.classList.contains('light') ? 'light' : null;
    t = window.SSI[S.lang];
    S.test = !!st.test;
    S.installed = st.installed || null;
    S.defaults = st.defaults || {};
    S.gpuInfo = st.gpu || null;
    S.opts = { root: S.defaults.root || '', data_dir: S.defaults.data_dir || '', gpu: gpuOk(), autostart: true, mft: true, desktop: false };
    await Promise.all([
      api.listen('setup://plan', onPlan), api.listen('setup://step', onStep),
      api.listen('setup://error', onError), api.listen('setup://done', onDone)
    ]);
    if (S.mode === 'install') await refreshDisk(false, true);
    S.scr = home();
    mount(true);
    if (api.autorun) api.autorun({ go, install: startInstall, update: startUpdate, uninstall: wipe => { S.wipe = !!wipe; startUninstall(); }, state: S });
  }

  boot();
})();

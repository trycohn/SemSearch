/* SemSearch Setup — fake core for a normal browser (loaded by app.js only when window.__TAURI__ is absent).
   index.html?mock=<state>[&lang=en][&dark=1|0][&gpu=none|old|driver][&wipe=1][&engfail=1]
   states: welcome, options, nospace, progress, neterror, error, done, noadmin, update, updated, uninstall, removed.
   Commands and events follow docs/INSTALLER.md; the run is a script of setup://plan|step|error|done events. */
(function () {
  'use strict';
  const q = new URLSearchParams(location.search);
  const scen = (q.get('mock') || 'welcome').toLowerCase();
  const html = document.documentElement;
  html.classList.add('mock');
  if (q.get('dark') === '1') html.classList.add('dark');
  else if (q.get('dark') === '0') html.classList.add('light');

  const GiB = 1073741824;
  const mode = (scen === 'update' || scen === 'updated') ? 'update' : (scen === 'uninstall' || scen === 'removed') ? 'uninstall' : 'install';
  const ROOT = 'C:\\Users\\Anna\\AppData\\Local\\Programs\\SemSearch', DATA = 'C:\\Users\\Anna\\AppData\\Local\\SemSearch';
  const gq = q.get('gpu');
  const gpu = gq === 'none' ? null
    : gq === 'old' ? { name: 'NVIDIA GeForce GTX 1060 6GB', cap: '6.1', supported: false, reason: 'cap' }
    : gq === 'driver' ? { name: 'NVIDIA GeForce RTX 3060', cap: '8.6', supported: false, reason: 'driver' }
    : { name: 'NVIDIA GeForce RTX 4070 SUPER', cap: '8.9', supported: true };
  const STATE = {
    mode, version: '0.5', lang: q.get('lang') === 'en' ? 'en' : 'ru', test: false,
    installed: mode === 'install' ? null : { version: '0.4', root: ROOT, data_dir: DATA, gpu: true, mft: true, autostart: true, desktop: false },
    defaults: { root: ROOT, data_dir: DATA }, gpu, windows: { build: 19045, ok: true }
  };

  const L = {};
  const emit = (name, payload) => (L[name] || []).slice().forEach(cb => cb(JSON.parse(JSON.stringify(payload))));
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const log = (...a) => console.log('[mock]', ...a);

  /* ---- the run: ops {ms, fn} | {halt, fn, resume} | {mark} ---- */
  let ops = [], pos = 0, halted = null, cancelled = false, cur = null, runOpts = null, runId = 0;
  const step = (id, state, extra) => () => { if (state === 'active') cur = id; emit('setup://step', Object.assign({ id, state }, extra || {})); };

  function build(o) {
    ops = []; pos = 0; halted = null; cancelled = false;
    const at = (ms, fn) => ops.push({ ms, fn });
    const plain = (id, ms) => { at(60, step(id, 'active')); at(ms, step(id, 'done')); };
    const bytes = (id, total, n, failAt) => {
      at(60, step(id, 'active', { got: 0, total }));
      let f = 0;
      for (let i = 1; i <= n; i++) {
        const next = i === n ? 1 : Math.min(1, i / n + (((i * 7919) % 11) - 5) / 300);   // uneven chunks
        f = Math.max(f, next);
        const got = Math.round(total * f);
        if (failAt != null && f >= failAt && failAt >= 0) {
          ops.push({
            halt: true,
            fn: () => {
              emit('setup://step', { id, state: 'active', got, total });
              emit('setup://step', { id, state: 'failed', got, total });
              emit('setup://error', { id, kind: 'network', message: 'connection reset by peer' });
            },
            resume: step(id, 'active', { got, total })
          });
          failAt = null;
          continue;
        }
        at(380, step(id, 'active', { got, total }));
        if (scen === 'progress' && id === 'torch' && i === Math.round(n * 0.45)) ops.push({ mark: 'mid' });
      }
      at(200, step(id, 'done', { got: total, total }));
    };

    if (mode === 'install') {
      const plan = ['check', 'copy', 'torch', 'libs', 'model'].concat(o.mft ? ['mft'] : [], ['links']);
      at(0, () => emit('setup://plan', { mode, steps: plan }));
      plain('check', 800);
      plain('copy', 1200);
      bytes('torch', Math.round((o.gpu ? 3.0 : 0.2) * GiB), o.gpu ? 14 : 4, scen === 'neterror' ? 0.42 : null);
      if (scen === 'error') {
        at(60, step('libs', 'active'));
        ops.push({
          halt: true,
          fn: () => {
            emit('setup://step', { id: 'libs', state: 'failed' });
            emit('setup://error', { id: 'libs', kind: 'other', message: 'pip завершился с кодом 1: ERROR: Could not install packages due to an OSError: [WinError 5] Отказано в доступе' });
          },
          resume: step('libs', 'active')
        });
        at(1600, step('libs', 'done'));
      } else plain('libs', 1600);
      bytes('model', Math.round(1.5 * GiB), 8, null);
      if (o.mft) {
        at(60, step('mft', 'active'));
        if (scen === 'noadmin') {
          at(1400, step('mft', 'failed'));
          at(0, () => emit('setup://error', { id: 'mft', kind: 'uac_denied', message: 'UAC declined' }));   // non-fatal: UI must not stop
        } else at(1400, step('mft', 'done'));
      }
      plain('links', 500);
      at(100, () => emit('setup://done', { mode, ok: true, mft_ok: !(o.mft && scen === 'noadmin'), root: o.root, data_dir: o.data_dir }));
    } else if (mode === 'update') {
      const plan = ['close', 'replace', 'libs', 'mft', 'links'];
      at(0, () => emit('setup://plan', { mode, steps: plan }));
      plain('close', 700); plain('replace', 1300); plain('libs', 1500);
      at(60, step('mft', 'active')); at(1200, step('mft', 'done'));
      plain('links', 400);
      at(100, () => emit('setup://done', { mode, ok: true, mft_ok: true, root: ROOT, data_dir: DATA }));
    } else {
      const plan = ['close', 'engine_del', 'prog_del'].concat(o.wipe ? ['data_del'] : []);
      at(0, () => emit('setup://plan', { mode, steps: plan }));
      plain('close', 700);
      at(60, step('engine_del', 'active'));
      at(1000, step('engine_del', q.get('engfail') === '1' ? 'failed' : 'done'));
      plain('prog_del', 1000);
      if (o.wipe) plain('data_del', 700);
      at(100, () => emit('setup://done', { mode, ok: true, mft_ok: true, root: ROOT, data_dir: DATA }));
    }
  }

  async function run(fastUntil) {
    const my = runId;
    let fast = !!fastUntil;
    while (pos < ops.length && my === runId && !cancelled) {
      const op = ops[pos++];
      if (op.mark) { if (fastUntil === op.mark) fast = false; continue; }
      if (op.halt) { halted = op; op.fn(); return; }
      if (!fast) await sleep(op.ms);
      if (my !== runId || cancelled) return;
      op.fn();
    }
  }

  /* ---- commands ---- */
  async function diskInfo(a) {
    await sleep(60);
    const need = (a.gpu ? 7 : 3.5) * GiB;
    const free = d => (d === 'C:' ? (scen === 'nospace' ? 3.1 : 214) : 812) * GiB;
    const drive = String(a.root || 'C:').slice(0, 2).toUpperCase(), dd = String(a.data_dir || 'C:').slice(0, 2).toUpperCase();
    const r = { drive, free_bytes: free(drive), need_bytes: need, ok: free(drive) >= need };
    if (dd !== drive) { r.data_drive = dd; r.data_free_bytes = free(dd); r.ok = r.ok && r.data_free_bytes >= 2 * GiB; }
    return r;
  }
  const CMD = {
    get_state: async () => { await sleep(30); return JSON.parse(JSON.stringify(STATE)); },
    disk_info: diskInfo,
    pick_folder: async a => {
      await sleep(150);
      const init = String(a.initial || '');
      if (/Programs/i.test(init)) return init.startsWith('C:') ? 'D:\\Programs' : ROOT;
      return init.startsWith('C:') ? 'D:\\SemSearch Data' : DATA;
    },
    start: async a => {
      log('start', a);
      runOpts = a; runId++;
      build(a);
      const ff = { neterror: true, error: true, done: true, noadmin: true, removed: true, updated: true }[scen] ? 'end' : scen === 'progress' ? 'mid' : null;
      run(ff);
      return null;
    },
    retry: async () => {
      log('retry');
      if (!halted) return null;
      const h = halted; halted = null;
      h.resume();
      run(null);
      return null;
    },
    cancel: async () => {
      log('cancel');
      cancelled = true; runId++;
      if (cur) emit('setup://error', { id: cur, kind: 'cancelled', message: 'cancelled' });
      return null;
    },
    retry_mft: async () => {
      log('retry_mft');
      emit('setup://step', { id: 'mft', state: 'active' });
      await sleep(1500);
      emit('setup://step', { id: 'mft', state: q.get('mftfail') === '2' ? 'failed' : 'done' });
      return null;
    },
    finish: async a => { log('finish', a); setTimeout(() => location.reload(), 500); return null; },
    open_log: async () => { log('open_log'); return null; },
    open_path: async a => { log('open_path', a); return null; }
  };

  window.SSMock = {
    real: false,
    invoke: (cmd, a) => (CMD[cmd] ? CMD[cmd](a || {}) : Promise.reject('mock: unknown command ' + cmd)),
    listen: (name, cb) => { (L[name] = L[name] || []).push(cb); return Promise.resolve(() => { L[name] = L[name].filter(f => f !== cb); }); },
    win: () => ({ minimize: () => log('minimize'), close: () => log('close') }),
    autorun(app) {
      if (scen === 'options') app.go('options');
      else if (['progress', 'neterror', 'error', 'done', 'noadmin'].includes(scen)) app.install();
      else if (scen === 'updated') app.update();
      else if (scen === 'removed') app.uninstall(q.get('wipe') === '1');
    }
  };
})();

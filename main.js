'use strict';
/**
 * Windows SMS Bridge — Electron main process.
 *
 * Responsibilities
 *  · window / tray / single-instance lifecycle
 *  · IPC surface for the renderer (no node access in the UI)
 *  · background sync scheduler, boot autostart, new-SMS notifications
 */

const {
  app, BrowserWindow, ipcMain, Tray, Menu, Notification, shell, dialog, safeStorage, nativeImage,
} = require('electron');
const path = require('node:path');
const fs = require('node:fs');

const { Store } = require('./src/core/store');
const { recoverProfile } = require('./src/core/migrate');
const i18n = require('./src/shared/i18n');
const t = (key, vars) => i18n.t(key, vars);
const { syncPhone } = require('./src/core/sync');
const {
  pair, health, normalizeBase, pinCertificate, prettyFingerprint, BridgeError,
} = require('./src/core/smsbridge');

const APP_NAME = 'Windows SMS Bridge';
const START_HIDDEN = process.argv.includes('--hidden');

app.setName(APP_NAME);

// Profile recovery across the rename: keeps pairings (and their DPAPI tokens)
// alive even if an earlier build already created an empty profile.
try {
  recoverProfile({ appData: app.getPath('appData'), appName: APP_NAME, log: console.log });
} catch (err) {
  console.warn('[migrate] failed', err);
}
if (process.platform === 'win32') app.setAppUserModelId('com.windowssmsbridge.windows');
// dev/testing: run against an isolated data directory
if (process.env.SMSGLASS_DATA_DIR) {
  app.setPath('userData', path.resolve(process.env.SMSGLASS_DATA_DIR));
}

let win = null;
let tray = null;
let store = null;
let scheduler = null;
let quitting = false;
let lastSyncSummary = null;

// --------------------------------------------------------------------- setup

const gotLock = app.requestSingleInstanceLock();
if (!gotLock) {
  app.quit();
} else {
  app.on('second-instance', () => showWindow());
}

function dataDir() {
  return app.getPath('userData');
}

/**
 * Display scale: the layout is built in px, so Electron's zoom factor is the
 * honest way to make everything bigger instead of fighting 200 font sizes.
 */
/**
 * Interface language lives in settings but the main process needs it too
 * (tray menu, notifications, dialogs).
 */
function applyLanguage(setting) {
  const lang = i18n.resolveLang(setting, app.getLocale());
  i18n.setLang(lang);
  refreshTrayMenu();
  return lang;
}

function applyUiScale(value) {
  const scale = Math.min(1.5, Math.max(0.85, Number(value) || 1));
  try { win?.webContents.setZoomFactor(scale); } catch { /* window not ready */ }
  return scale;
}

function bootStore() {
  store = new Store({ dir: dataDir(), safeStorage }).load();
  store.prune();
  applyLanguage(store.settings.lang);
}

function createWindow() {
  const { screen } = require('electron');
  const area = screen.getPrimaryDisplay().workAreaSize;
  const width = Number(process.env.SMSGLASS_WIN_W) || Math.min(1380, Math.max(1000, area.width - 40));
  const height = Number(process.env.SMSGLASS_WIN_H) || Math.min(880, Math.max(660, area.height - 30));

  win = new BrowserWindow({
    width,
    height,
    minWidth: 1000,
    minHeight: 640,
    show: false,
    frame: false,
    backgroundColor: '#07070b',
    titleBarStyle: 'hidden',
    autoHideMenuBar: true,
    icon: iconPath(),
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      spellcheck: false,
      // keep the renderer painting when the window is hidden in the tray or
      // covered by another app, otherwise timers/animations freeze and the
      // window shows a stale frame when it comes back
      backgroundThrottling: false,
    },
  });

  win.loadFile(path.join(__dirname, 'src', 'ui', 'index.html'));
  win.webContents.on('did-finish-load', () => applyUiScale(store.settings.uiScale));

  win.once('ready-to-show', () => {
    if (!START_HIDDEN && !store.settings.startHidden) {
      win.show();
    }
  });

  win.on('close', (e) => {
    if (!quitting && store.settings.closeToTray) {
      e.preventDefault();
      win.hide();
    }
  });

  win.on('maximize', () => send('window:maximized', true));
  win.on('unmaximize', () => send('window:maximized', false));

  // dev helper: SMSGLASS_SHOT=<png> [SMSGLASS_SHOT_JS=<js>] [SMSGLASS_SHOT_DELAY=ms]
  if (process.env.SMSGLASS_SHOT) {
    win.webContents.once('did-finish-load', () => {
      const delay = Number(process.env.SMSGLASS_SHOT_DELAY || 1600);
      setTimeout(async () => {
        try {
          if (process.env.SMSGLASS_SHOT_JS) {
            const out = await win.webContents.executeJavaScript(process.env.SMSGLASS_SHOT_JS, true);
            console.log('[shot:js]', typeof out === 'string' ? out : JSON.stringify(out));
            await new Promise((r) => setTimeout(r, 700));
          }
          // screenshots must not catch a half-played animation
          await win.webContents.executeJavaScript(
            "(() => { const s = document.createElement('style'); s.id='__shot';"
            + "s.textContent = '*,*::before,*::after{animation:none!important;transition:none!important}';"
            + "document.head.appendChild(s); void document.body.offsetHeight; })()",
            true,
          );
          await new Promise((r) => setTimeout(r, 250));
          const img = await win.webContents.capturePage();
          fs.writeFileSync(process.env.SMSGLASS_SHOT, img.toPNG());
          console.log('[shot]', process.env.SMSGLASS_SHOT);
        } catch (err) {
          console.error('[shot]', err);
        }
        if (process.env.SMSGLASS_SHOT_EXIT) app.quit();
      }, delay);
    });
  }

  // external links open in the default browser
  win.webContents.setWindowOpenHandler(({ url }) => {
    shell.openExternal(url);
    return { action: 'deny' };
  });
}

function showWindow() {
  if (!win) return createWindow();
  if (win.isMinimized()) win.restore();
  win.show();
  win.focus();
}

function iconPath() {
  const ico = path.join(__dirname, 'assets', 'icon.ico');
  const png = path.join(__dirname, 'assets', 'icon.png');
  return fs.existsSync(ico) ? ico : png;
}

function send(channel, payload) {
  if (win && !win.isDestroyed()) win.webContents.send(channel, payload);
}

// ---------------------------------------------------------------------- tray

function trayImage() {
  // An .ico carries the sizes Windows asks for (16/20/24/32) and stays crisp at
  // any DPI; the PNG is a fallback, and the app icon a last resort so the tray
  // never ends up as an empty square.
  const candidates = ['tray.ico', 'tray.png', 'icon.png'];
  for (const name of candidates) {
    const file = path.join(__dirname, 'assets', name);
    if (!fs.existsSync(file)) continue;
    const img = nativeImage.createFromPath(file);
    if (img.isEmpty()) continue;
    return name.endsWith('.ico') ? img : img.resize({ width: 16, height: 16 });
  }
  console.warn('[tray] no icon file found next to', __dirname);
  return nativeImage.createEmpty();
}

function buildTray() {
  if (tray) return;
  tray = new Tray(trayImage());
  tray.setToolTip(APP_NAME);
  refreshTrayMenu();

  tray.on('click', () => showWindow());
  tray.on('double-click', () => showWindow());
}

function refreshTrayMenu() {
  if (!tray) return;
  const phones = store ? store.listPhones() : [];
  const syncing = syncState.running ? t('同步中…') : t('立即同步全部');
  const menu = Menu.buildFromTemplate([
    { label: APP_NAME, enabled: false },
    { type: 'separator' },
    { label: t('已配对手机 · {n}', { n: phones.length }), enabled: false },
    ...phones.slice(0, 6).map((p) => ({
      label: `  ${p.name}  ·  ${p.url.replace(/^https?:\/\//, '')}`,
      click: () => { showWindow(); syncOne(p.id, true); },
    })),
    { type: 'separator' },
    { label: syncing, enabled: !syncState.running, click: () => syncAll({ manual: true }) },
    { label: t('打开主界面'), click: () => showWindow() },
    {
      label: t('开机自启'),
      type: 'checkbox',
      checked: Boolean(store?.settings.autoStart),
      click: (item) => {
        applyAutoStart(item.checked);
        send('settings:update', store.settings);
      },
    },
    { type: 'separator' },
    { label: t('退出'), click: () => { quitting = true; app.quit(); } },
  ]);
  tray.setContextMenu(menu);
  tray.setToolTip(`${APP_NAME} · ${t('{n} 台手机', { n: phones.length })}${syncState.lastAt ? t(' · 已同步') : ''}`);
}

// ----------------------------------------------------------------- autostart
//
// We manage the per-user Run key ourselves instead of app.setLoginItemSettings:
// Electron on Windows registers the entry under the AppUserModelId, which is easy
// to duplicate and impossible to read back reliably. One predictable value only.

const RUN_KEY = 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run';

function reg(args) {
  try {
    require('node:child_process').execFile('reg', args, { windowsHide: true }, () => {});
  } catch { /* best effort */ }
}

function autostartValue() {
  const args = app.isPackaged ? ['--hidden'] : [app.getAppPath(), '--hidden'];
  return `"${process.execPath}" ${args.map((a) => `"${a}"`).join(' ')}`;
}

function applyAutoStart(enabled) {
  store.updateSettings({ autoStart: Boolean(enabled) });
  if (process.platform === 'win32') {
    if (enabled) {
      reg(['add', RUN_KEY, '/v', APP_NAME, '/t', 'REG_SZ', '/d', autostartValue(), '/f']);
    } else {
      reg(['delete', RUN_KEY, '/v', APP_NAME, '/f']);
    }
    // clean up entries left by older builds / the old app name
    reg(['delete', RUN_KEY, '/v', 'com.smsglass.app', '/f']);
    reg(['delete', RUN_KEY, '/v', 'SMS Glass', '/f']);
  }
  refreshTrayMenu();
  return store.settings;
}

// --------------------------------------------------------------------- sync

const syncState = { running: false, lastAt: 0, lastSummary: null, errors: [] };

async function syncOne(phoneId, manual = false) {
  const phone = store.getPhone(phoneId);
  if (!phone) return null;
  try {
    const res = await syncPhone(store, phoneId, {
      full: manual && manual !== 'incremental',
      onProgress: (p) => send('sync:progress', p),
    });
    await refreshHealth(phoneId);
    afterSync([res]);
    return res;
  } catch (err) {
    // A revoked / stale token is not a network problem: say so, and let the UI
    // offer "重新配对" instead of a generic red dot.
    const unauthorized = err.status === 401
      || err.code === 'unauthorized'
      || /unknown client|bad signature|revoked/i.test(err.message || '');
    const message = unauthorized
      ? t('配对凭据已失效：右键这台设备选「重新配对（换新密钥/证书）」')
      : (err.message || String(err));
    store.updatePhone(phoneId, { lastError: message, authError: unauthorized });
    syncState.errors.push({ phoneId, message });
    send('sync:error', { phoneId, message: err.message, code: err.code });
    refreshTrayMenu();
    if (manual) throw err;
    return null;
  }
}

async function syncAll({ manual = false, full = false } = {}) {
  if (syncState.running) return syncState.lastSummary;
  const phones = store.listPhones();
  if (!phones.length) {
    if (manual) throw new Error('还没有配对任何手机');
    return null;
  }
  syncState.running = true;
  syncState.errors = [];
  send('sync:started', { count: phones.length });
  refreshTrayMenu();

  const results = [];
  try {
    for (const p of phones) {
      try {
        const res = await syncPhone(store, p.id, {
          full,
          days: store.settings.pullDays,
          onProgress: (pr) => send('sync:progress', pr),
        });
        results.push(res);
        await refreshHealth(p.id);
      } catch (err) {
        syncState.errors.push({ phoneId: p.id, name: p.name, message: err.message });
        store.updatePhone(p.id, { lastError: err.message });
        send('sync:error', { phoneId: p.id, name: p.name, message: err.message, code: err.code });
      }
    }
  } finally {
    syncState.running = false;
    afterSync(results);
  }

  if (manual && !results.length && syncState.errors.length) {
    const e = syncState.errors[0];
    throw new BridgeError(e.message);
  }
  return syncState.lastSummary;
}

function afterSync(results) {
  syncState.lastAt = Date.now();
  const added = results.reduce((n, r) => n + (r.added || 0), 0);
  syncState.lastSummary = {
    at: syncState.lastAt,
    added,
    phones: results.length,
    errors: syncState.errors.slice(),
  };
  store.prune();
  notifyNewMessages(added, results);
  refreshTrayMenu();
  send('state:changed', { reason: 'sync', summary: syncState.lastSummary });
}

async function refreshHealth(phoneId) {
  const phone = store.getPhone(phoneId);
  if (!phone) return null;
  try {
    const h = await health(phone.url, { certPem: phone.certPem, timeout: 6000 });
    store.updatePhone(phoneId, { health: { ...h, fetchedAt: Date.now() }, lastError: '' });
    return h;
  } catch (err) {
    store.updatePhone(phoneId, { health: null, lastError: err.message });
    return null;
  }
}

function notifyNewMessages(added, results) {
  if (!added || !store.settings.notifyNew || !Notification.isSupported()) return;
  const fresh = [];
  for (const r of results) {
    if (!r.added) continue;
    const list = store.messagesOf(r.phoneId).filter((m) => m.type === 'inbox').slice(0, 3);
    fresh.push(...list);
  }
  if (!fresh.length) return;
  const first = fresh[0];
  if (!win || !win.isVisible()) {
    new Notification({
      title: added === 1
        ? t('新短信 · {addr}', { addr: first.address })
        : t('{n} 条新短信', { n: added }),
      body: (first.body || '').slice(0, 120),
      silent: false,
    }).show();
  }
}

function startScheduler() {
  if (scheduler) clearInterval(scheduler);
  scheduler = null;
  const minutes = Number(store.settings.syncIntervalMin) || 0;
  if (minutes <= 0) return;
  scheduler = setInterval(() => {
    if (!syncState.running) syncAll({ manual: false });
  }, Math.max(1, minutes) * 60000);
}

// ---------------------------------------------------------------------- IPC

function snapshot() {
  const phones = store.listPhones();
  const messages = store.allMessages().slice(0, 20000);
  return {
    phones,
    messages,
    settings: store.settings,
    stats: store.stats(),
    sync: { ...syncState },
    app: { version: app.getVersion(), dataDir: dataDir(), autostart: Boolean(store.settings.autoStart) },
  };
}

function registerIpc() {
  ipcMain.handle('state:get', () => snapshot());

  ipcMain.handle('window:minimize', () => win?.minimize());
  ipcMain.handle('window:toggle-maximize', () => {
    if (!win) return false;
    if (win.isMaximized()) win.unmaximize();
    else win.maximize();
    return win.isMaximized();
  });
  ipcMain.handle('window:close', () => {
    if (store.settings.closeToTray) win?.hide();
    else { quitting = true; app.quit(); }
  });
  ipcMain.handle('window:is-maximized', () => Boolean(win?.isMaximized()));

  // TOFU probe: fetch the phone certificate and report its fingerprint so the
  // user can compare it with the one shown on the phone's pairing screen.
  ipcMain.handle('phone:probe', async (_e, url) => {
    const pin = await pinCertificate(url, 8000);
    const h = await health(pin.base, { certPem: pin.pem, timeout: 6000 });
    return { ...h, base: pin.base, fingerprint: pin.pretty, fingerprintHex: pin.fingerprint };
  });

  ipcMain.handle('phone:pair', async (_e, { name, url, code, phoneId }) => {
    const pin = await pinCertificate(url, 10000);
    const info = await health(pin.base, { certPem: pin.pem, timeout: 6000 }); // fail fast
    const creds = await pair({
      baseUrl: pin.base,
      code,
      name: name || 'Windows SMS Bridge',
      certPem: pin.pem,
    });

    // re-pair: keep the record (and its cached history), swap only the credentials
    let id = phoneId;
    if (id && store.getPhone(id)) {
      store.updatePhone(id, {
        url: creds.baseUrl,
        clientId: creds.clientId,
        clientName: creds.clientName,
        certPem: pin.pem,
        certFp: pin.fingerprint,
        health: { ...info, fetchedAt: Date.now() },
        lastError: '',
        authError: false,
        tokenRaw: '',
        ...store.sealToken(creds.token),
      });
    } else {
      const phone = store.addPhone({
        name: name || info.model || '我的手机',
        url: creds.baseUrl,
        clientId: creds.clientId,
        token: creds.token,
        clientName: creds.clientName,
        certPem: pin.pem,
        certFp: pin.fingerprint,
      });
      id = phone.id;
      store.updatePhone(id, { health: { ...info, fetchedAt: Date.now() } });
    }

    buildTrayMenuSafe();
    // first backfill
    try {
      await syncPhone(store, id, { full: true, days: store.settings.pullDays });
      await refreshHealth(id);
      send('state:changed', { reason: 'paired' });
    } catch (err) {
      send('state:changed', { reason: 'paired' });
    }
    const phone = store.listPhones().find((p) => p.id === id);
    return { ...phone, fingerprint: pin.pretty, fingerprintHex: pin.fingerprint };
  });

  ipcMain.handle('phone:import', async (_e, { name, url, clientId, token }) => {
    const pin = await pinCertificate(url, 10000);
    // validate by a signed call, over the pinned certificate only
    const { BridgeClient } = require('./src/core/smsbridge');
    const who = await new BridgeClient({
      baseUrl: pin.base, clientId, token, certPem: pin.pem,
    }).whoami();
    const phone = store.addPhone({
      name: name || '我的手机',
      url: pin.base,
      clientId,
      token,
      clientName: who?.name || '',
      certPem: pin.pem,
      certFp: pin.fingerprint,
    });
    await refreshHealth(phone.id);
    buildTrayMenuSafe();
    try {
      await syncPhone(store, phone.id, { full: true, days: store.settings.pullDays });
    } catch { /* keep the phone even if the first sync fails */ }
    send('state:changed', { reason: 'imported' });
    return { ...store.listPhones().find((p) => p.id === phone.id), fingerprint: pin.pretty };
  });

  ipcMain.handle('phone:remove', (_e, id) => {
    store.removePhone(id);
    buildTrayMenuSafe();
    return store.listPhones();
  });

  ipcMain.handle('phone:rename', (_e, { id, name }) => store.updatePhone(id, { name }));

  ipcMain.handle('phone:health', async (_e, id) => {
    const h = await refreshHealth(id);
    send('state:changed', { reason: 'health' });
    return h;
  });

  ipcMain.handle('sync:one', async (_e, { id }) => {
    const res = await syncOne(id, 'incremental');
    return { result: res, state: snapshot() };
  });

  ipcMain.handle('sync:all', async () => {
    await syncAll({ manual: true });
    return { state: snapshot(), summary: syncState.lastSummary };
  });

  ipcMain.handle('settings:set', (_e, patch) => {
    if ('autoStart' in patch) applyAutoStart(patch.autoStart);
    const next = store.updateSettings(patch);
    if ('syncIntervalMin' in patch) startScheduler();
    if ('uiScale' in patch) applyUiScale(next.uiScale);
    if ('lang' in patch) applyLanguage(next.lang);
    send('settings:update', next);
    return next;
  });

  ipcMain.handle('messages:seen', (_e, address) => store.markSeen(address));

  ipcMain.handle('messages:clear', (_e, phoneId) => {
    store.clearMessages(phoneId);
    return snapshot();
  });

  ipcMain.handle('messages:export', async (_e, { phoneId } = {}) => {
    const { canceled, filePath } = await dialog.showSaveDialog(win, {
      title: '导出短信',
      defaultPath: path.join(app.getPath('documents'), `sms-export-${Date.now()}.csv`),
      filters: [{ name: 'CSV', extensions: ['csv'] }],
    });
    if (canceled || !filePath) return { saved: false };
    const rows = (phoneId ? store.messagesOf(phoneId) : store.allMessages());
    const esc = (v) => `"${String(v ?? '').replace(/"/g, '""')}"`;
    const lines = [[t('时间'), t('号码'), t('方向'), t('卡槽'), t('运营商'), t('内容')].join(',')];
    for (const m of rows) {
      lines.push([new Date(m.date).toLocaleString(i18n.getLang() === 'zh' ? 'zh-CN' : i18n.getLang() === 'ja' ? 'ja-JP' : 'en-US'), m.address, m.type === 'inbox' ? t('收') : t('发'), m.slot, m.carrier, m.body].map(esc).join(','));
    }
    fs.writeFileSync(filePath, '\ufeff' + lines.join('\r\n'), 'utf8');
    return { saved: true, filePath, count: rows.length };
  });

  ipcMain.handle('app:open-data-dir', () => shell.openPath(dataDir()));
  ipcMain.handle('app:open-external', (_e, url) => {
    if (typeof url === 'string' && /^https?:\/\//i.test(url)) return shell.openExternal(url);
    return false;
  });
  ipcMain.handle('app:reveal-config', () => shell.showItemInFolder(path.join(dataDir(), 'config.json')));
}

function buildTrayMenuSafe() {
  try { refreshTrayMenu(); } catch { /* tray not ready yet */ }
}

// -------------------------------------------------------------------- boot

app.whenReady().then(async () => {
  bootStore();
  registerIpc();

  createWindow();
  buildTray();
  startScheduler();

  const first = store.listPhones()[0];
  if (first) setTimeout(() => refreshHealth(first.id), 2500);

  if (store.settings.syncOnStart && store.listPhones().length) {
    setTimeout(() => { if (!syncState.running) syncAll({ manual: false }); }, 4000);
  }

  app.on('activate', () => showWindow());
});

app.on('before-quit', () => {
  quitting = true;
  if (scheduler) clearInterval(scheduler);
  if (store) store.flushDb();
});

app.on('window-all-closed', () => {
  // tray-first app: stay alive unless the user really quits
  if (process.platform !== 'darwin' && quitting) app.quit();
});

process.on('uncaughtException', (err) => {
  console.error('[uncaught]', err);
});

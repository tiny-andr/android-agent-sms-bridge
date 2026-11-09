/**
 * Fake renderer bridge for scripts/i18n-sweep.js.
 *
 * The real app talks to the main process through window.sms (see preload.js).
 * The sweep only needs the UI to render, so every call is answered from an
 * in-memory dataset. Dataset comes from --sweep-ds=<empty|paired> (per window)
 * via webPreferences.additionalArguments.
 */
const { contextBridge } = require('electron');

const dsArg = process.argv.find((a) => a.startsWith('--sweep-ds='));
const DATASET = dsArg && dsArg.split('=')[1] === 'empty' ? 'empty' : 'paired';

const SETTINGS = {
  theme: 'dark',
  lang: 'zh',
  uiScale: 1,
  autoStart: false,
  startHidden: false,
  closeToTray: true,
  syncOnStart: false,
  notifyNew: true,
  syncIntervalMin: 15,
  pullDays: 30,
  keepDays: 0,
};

const PHONES = [
  {
    id: 'sweep-phone-1',
    name: 'Pixel 9',
    url: 'http://192.168.1.5:8787',
    lastSyncAt: Date.now() - 6 * 60000,
    health: { ok: true, model: 'Pixel 9', battery: 76, charging: false, sims: [{ carrier: 'Carrier A' }] },
  },
  {
    id: 'sweep-phone-2',
    name: 'Mi 14',
    url: 'http://192.168.1.6:8787',
    lastSyncAt: Date.now() - 60 * 60000,
    health: { ok: true, model: 'Mi 14', battery: 41, charging: false, sims: [{ carrier: 'Carrier B' }] },
  },
];

const MESSAGES = [
  { id: 1, phoneId: 'sweep-phone-1', address: '+8613800138000', body: 'On my way, six thirty works.', date: Date.now() - 4 * 3600000, type: 'inbox', read: false, seen: false },
  { id: 2, phoneId: 'sweep-phone-1', address: '+8613800138000', body: 'Great, see you then.', date: Date.now() - 3 * 3600000, type: 'sent', read: true, seen: true },
  { id: 3, phoneId: 'sweep-phone-1', address: '95588', body: 'Your account balance is 8823 at 14:20.', date: Date.now() - 26 * 3600000, type: 'inbox', read: true, seen: true },
  { id: 4, phoneId: 'sweep-phone-2', address: '10086', body: 'Your data plan is 68 percent used.', date: Date.now() - 2 * 86400000, type: 'inbox', read: true, seen: true },
];

const snapshot = () => ({
  phones: DATASET === 'empty' ? [] : PHONES,
  messages: DATASET === 'empty' ? [] : MESSAGES,
  settings: { ...SETTINGS },
  stats: { total: DATASET === 'empty' ? 0 : MESSAGES.length },
  sync: { running: false, lastAt: null },
  app: { version: '1.0.0', dataDir: 'C:/Users/Sweep/AppData/Windows SMS Bridge', autostart: false },
});

// contextBridge objects are read-only from the page, so behaviour is switched
// through this hook instead of assigning window.sms.pair
const behaviour = { pair: 'reject' };

const bridge = {
  getState: async () => snapshot(),
  pair: async () => {
    if (behaviour.pair === 'pending') return new Promise(() => {});
    throw new Error('sweep: pairing disabled');
  },
  importPhone: async () => { throw new Error('sweep: import disabled'); },
  probe: async () => ({ ok: false, error: 'sweep' }),
  removePhone: async () => {},
  renamePhone: async () => {},
  phoneHealth: async () => ({}),
  syncOne: async () => ({}),
  syncAll: async () => ({}),
  setSettings: async (patch) => {
    Object.assign(SETTINGS, patch);
    return { ...SETTINGS };
  },
  clearMessages: async () => {},
  markSeen: async () => 0,
  exportMessages: async () => ({ saved: false }),
  openDataDir: async () => {},
  openExternal: async () => {},
  revealConfig: async () => {},
  win: {
    minimize: async () => {},
    toggleMaximize: async () => {},
    close: async () => {},
    isMaximized: async () => false,
  },
};

// event subscribers: the sweep never fires them, but app.js must be able to bind
for (const name of ['onStateChanged', 'onSyncStarted', 'onSyncProgress', 'onSyncError', 'onSettingsUpdate', 'onMaximized']) {
  bridge[name] = () => () => {};
}

contextBridge.exposeInMainWorld('sms', bridge);
contextBridge.exposeInMainWorld('sweepInfo', {
  dataset: DATASET,
  // keep the fake main-process settings in sync with the renderer's language,
  // otherwise a later getState() would flip the UI back to the boot language
  setLang: (lang) => { SETTINGS.lang = lang; },
  setPairMode: (mode) => { behaviour.pair = mode; },
});

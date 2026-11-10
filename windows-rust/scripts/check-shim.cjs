#!/usr/bin/env node
/**
 * check-shim.cjs — proves web/ui/tauri.js matches the old Electron preload
 * contract (windows/preload.js) exactly, without Electron or a browser.
 *
 *   node scripts/check-shim.cjs     # exit 0 = pass
 *
 * Loads the shim into a vm context with a fake `window.__TAURI__` that records
 * every (channel, args) passed to core.invoke and every event.listen channel,
 * then walks an embedded expected table:
 *
 *   - 18 backend methods + 4 win.* methods: channel + argument object
 *   - 7 event subscribers: channel name + unlisten return value
 *   - rejection normalisation (bare-string Err -> Error with .message)
 *   - re-entrancy guard (pre-existing window.sms / no __TAURI__ / double load)
 *   - frameless-titlebar drag + dblclick wiring
 */
'use strict';

const fs = require('fs');
const path = require('path');
const vm = require('vm');

const SHIM_PATH = path.join(__dirname, '..', 'web', 'ui', 'tauri.js');
const SOURCE = fs.readFileSync(SHIM_PATH, 'utf8');

/* ------------------------------------------------------------- fixtures ── */

// Mirrors the table in the port spec / windows/preload.js. `args` is exactly
// what the Rust command must be able to deserialize.
const METHODS = [
  { path: ['getState'], call: [], channel: 'state:get', args: undefined },
  {
    path: ['pair'],
    call: [{ name: 'Pixel 9', url: 'http://192.168.1.5:8787', code: '123456', phoneId: 'phone-a' }],
    channel: 'phone:pair',
    args: { name: 'Pixel 9', url: 'http://192.168.1.5:8787', code: '123456', phoneId: 'phone-a' },
  },
  {
    path: ['importPhone'],
    call: [{ name: 'Mi 14', url: 'http://192.168.1.6:8787', clientId: 'client-1', token: 'tok-1' }],
    channel: 'phone:import',
    args: { name: 'Mi 14', url: 'http://192.168.1.6:8787', clientId: 'client-1', token: 'tok-1' },
  },
  { path: ['probe'], call: ['http://192.168.1.5:8787'], channel: 'phone:probe', args: { url: 'http://192.168.1.5:8787' } },
  { path: ['removePhone'], call: ['phone-a'], channel: 'phone:remove', args: { id: 'phone-a' } },
  { path: ['renamePhone'], call: ['phone-a', 'Bedroom phone'], channel: 'phone:rename', args: { id: 'phone-a', name: 'Bedroom phone' } },
  { path: ['phoneHealth'], call: ['phone-a'], channel: 'phone:health', args: { id: 'phone-a' } },
  { path: ['syncOne'], call: ['phone-a'], channel: 'sync:one', args: { id: 'phone-a' } },
  { path: ['syncAll'], call: [], channel: 'sync:all', args: undefined },
  { path: ['updateCheck'], call: [], channel: 'update:check', args: undefined },
  { path: ['updateApply'], call: [], channel: 'update:apply', args: undefined },
  { path: ['setSettings'], call: [{ lang: 'ja' }], channel: 'settings:set', args: { patch: { lang: 'ja' } } },
  { path: ['clearMessages'], call: ['phone-a'], channel: 'messages:clear', args: { phoneId: 'phone-a' } },
  { path: ['markSeen'], call: ['+8613800138000'], channel: 'messages:seen', args: { address: '+8613800138000' } },
  { path: ['exportMessages'], call: [null], channel: 'messages:export', args: { phoneId: null } },
  { path: ['openDataDir'], call: [], channel: 'app:open-data-dir', args: undefined },
  { path: ['openExternal'], call: ['https://example.test/x'], channel: 'app:open-external', args: { url: 'https://example.test/x' } },
  { path: ['revealConfig'], call: [], channel: 'app:reveal-config', args: undefined },
  { path: ['win', 'minimize'], call: [], channel: 'window:minimize', args: undefined },
  { path: ['win', 'toggleMaximize'], call: [], channel: 'window:toggle-maximize', args: undefined },
  { path: ['win', 'close'], call: [], channel: 'window:close', args: undefined },
  { path: ['win', 'isMaximized'], call: [], channel: 'window:is-maximized', args: undefined },
];

const EVENTS = [
  ['onStateChanged', 'state:changed'],
  ['onSyncProgress', 'sync:progress'],
  ['onSyncStarted', 'sync:started'],
  ['onSyncError', 'sync:error'],
  ['onUpdateProgress', 'update:progress'],
  ['onSettingsUpdate', 'settings:update'],
  ['onMaximized', 'window:maximized'],
];

/* --------------------------------------------------------------- harness ── */

const failures = [];
function fail(label, detail) {
  failures.push({ label, detail });
}
function ok(cond, label, detail) {
  if (!cond) fail(label, detail || 'expected truthy, got ' + JSON.stringify(cond));
}
function eq(actual, expected, label) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) fail(label, `  expected: ${e}\n  actual:   ${a}`);
}

const flush = async () => {
  for (let i = 0; i < 6; i += 1) await Promise.resolve();
};

function makeEnv(options) {
  options = options || {};
  const state = {
    calls: [],
    listeners: [],
    drags: [],
    unlistenCalls: 0,
    behaviour: { reject: false, rejectValue: null, throwValue: null },
  };

  const titlebarHandlers = {};
  const fakeTitlebar = {
    addEventListener(type, fn) {
      (titlebarHandlers[type] || (titlebarHandlers[type] = [])).push(fn);
    },
  };
  const fakeDocument = {
    querySelector(selector) {
      return selector === '.titlebar' ? fakeTitlebar : null;
    },
  };

  const fakeWindow = {};
  if (options.withSms) fakeWindow.sms = options.withSms;
  if (!options.withoutTauri) {
    fakeWindow.__TAURI__ = {
      core: {
        invoke(channel, args) {
          state.calls.push({ channel, args });
          if (state.behaviour.throwValue !== null) throw state.behaviour.throwValue;
          if (state.behaviour.reject) return Promise.reject(state.behaviour.rejectValue);
          return Promise.resolve({ ok: true, channel });
        },
      },
      event: {
        listen(channel, handler) {
          state.listeners.push({ channel, handler });
          return Promise.resolve(function unlisten() {
            state.unlistenCalls += 1;
          });
        },
      },
      window: {
        getCurrentWindow() {
          return {
            startDragging() {
              state.drags.push(1);
              return Promise.resolve();
            },
          };
        },
      },
    };
  }

  const sandbox = {
    window: fakeWindow,
    document: fakeDocument,
    setTimeout,
    clearTimeout,
    Promise,
    Error,
    String,
    Object,
    JSON,
    console,
  };
  const context = vm.createContext(sandbox);

  return {
    fakeWindow,
    fakeDocument,
    titlebarHandlers,
    state,
    run() {
      return vm.runInContext(SOURCE, context, { filename: 'web/ui/tauri.js' });
    },
  };
}

function rustCommand(channel) {
  return String(channel).replace(/[:-]/g, '_');
}

function methodAt(sms, pathParts) {
  return pathParts.reduce((obj, key) => (obj == null ? undefined : obj[key]), sms);
}

/* ------------------------------------------------------------------ tests ── */

async function checkContract() {
  const env = makeEnv();
  env.run();
  const sms = env.fakeWindow.sms;
  ok(sms && typeof sms === 'object', 'window.sms is defined after load');
  if (!sms) return;

  // every table method exists and is callable
  for (const entry of METHODS) {
    const fn = methodAt(sms, entry.path);
    ok(typeof fn === 'function', `method ${entry.path.join('.')} is a function`);
  }

  // exact top-level shape: 18 backend methods + win + 7 event helpers, no extras
  const expectedTop = METHODS.filter((entry) => entry.path.length === 1)
    .map((entry) => entry.path[0])
    .concat(['win'], EVENTS.map((pair) => pair[0]))
    .sort();
  eq(Object.keys(sms).sort(), expectedTop, 'window.sms top-level shape');

  // every call maps to exactly one channel with the right argument object
  for (const entry of METHODS) {
    const label = entry.path.join('.');
    env.state.calls.length = 0;
    const fn = methodAt(sms, entry.path);
    let threw = null;
    try {
      await fn.apply(sms, entry.call);
    } catch (err) {
      threw = err;
    }
    ok(!threw, `${label} resolves`, threw && threw.message);
    ok(env.state.calls.length === 1, `${label} invokes exactly once`, `got ${env.state.calls.length} invoke call(s)`);
    const record = env.state.calls[0] || {};
    // The renderer-facing channel keeps its Electron name; Tauri derives the
    // command name from the Rust identifier, so `state:get` → `state_get`.
    // The translation is mechanical and asserted here, one channel per command.
    eq(record.channel, rustCommand(entry.channel), `${label} command`);
    eq(record.args, entry.args, `${label} args`);
  }

  // app.js passes phoneId: null for "pair another phone" — must survive
  env.state.calls.length = 0;
  await sms.pair({ name: 'Mi 14', url: 'http://192.168.1.6:8787', code: '654321', phoneId: null });
  eq(env.state.calls[0] && env.state.calls[0].args, {
    name: 'Mi 14',
    url: 'http://192.168.1.6:8787',
    code: '654321',
    phoneId: null,
  }, 'pair with phoneId:null args');

  // win.* methods recorded on the same bridge
  eq(Object.keys(sms.win || {}).sort(), ['close', 'isMaximized', 'minimize', 'toggleMaximize'], 'win.* shape');
}

async function checkEvents() {
  const env = makeEnv();
  env.run();
  const sms = env.fakeWindow.sms;

  const channels = [];
  for (const [helper, channel] of EVENTS) {
    ok(typeof sms[helper] === 'function', `${helper} is a function`);
    env.state.listeners.length = 0;
    const before = env.state.unlistenCalls;
    const seen = [];
    const unlisten = sms[helper](function (payload) {
      seen.push(payload);
    });
    ok(typeof unlisten === 'function', `${helper} returns an unlisten function`);
    ok(env.state.listeners.length === 1, `${helper} registers exactly one listener`);
    const record = env.state.listeners[0] || {};
    eq(record.channel, channel, `${helper} channel`);
    channels.push(record.channel);

    // handler forwards evt.payload exactly like preload's listener()
    const payload = { probe: helper };
    record.handler({ event: channel, id: 1, payload });
    eq(seen, [payload], `${helper} callback receives e.payload`);
    eq(env.state.unlistenCalls, before, `${helper} unlisten promise not consumed yet`);

    await flush();
    unlisten();
    await flush();
    eq(env.state.unlistenCalls, before + 1, `${helper} unlisten() calls Tauri unlisten`);
  }

  // app.js binds all seven distinct channels at startup
  eq(channels.slice().sort(), EVENTS.map((pair) => pair[1]).sort(), 'seven distinct event channels');
}

async function checkRejectionNormalisation() {
  const env = makeEnv();
  env.run();
  const sms = env.fakeWindow.sms;

  // Rust `Err(String)` rejects with a bare string; app.js reads err.message
  env.state.behaviour.reject = true;
  env.state.behaviour.rejectValue = 'phone offline: connection refused';
  let caught = null;
  try {
    await sms.syncOne('phone-a');
  } catch (err) {
    caught = err;
  }
  ok(caught instanceof Error, 'string rejection becomes an Error');
  eq(caught && caught.message, 'phone offline: connection refused', 'string rejection message');

  // Tauri may also reject with { message } / an Error instance
  env.state.behaviour.rejectValue = { message: 'pair code expired' };
  caught = null;
  try {
    await sms.pair({ name: 'x', url: 'http://x', code: '000000', phoneId: null });
  } catch (err) {
    caught = err;
  }
  ok(caught instanceof Error, 'object rejection becomes an Error');
  eq(caught && caught.message, 'pair code expired', 'object rejection message');

  env.state.behaviour.rejectValue = 42;
  caught = null;
  try {
    await sms.getState();
  } catch (err) {
    caught = err;
  }
  ok(caught instanceof Error, 'non-string rejection becomes an Error');
  eq(caught && caught.message, '42', 'non-string rejection message');

  // synchronous throw from invoke() (bridge not ready) is normalised too
  env.state.behaviour.reject = false;
  env.state.behaviour.throwValue = 'ipc not ready';
  caught = null;
  try {
    await sms.getState();
  } catch (err) {
    caught = err;
  }
  ok(caught instanceof Error, 'synchronous invoke throw becomes an Error');
  eq(caught && caught.message, 'ipc not ready', 'synchronous invoke throw message');
}

async function checkGuards() {
  // 1) the Electron i18n sweep injects its own window.sms before page scripts:
  //    tauri.js must leave it untouched and not call IPC
  const injected = { injectedStub: true };
  const sweep = makeEnv({ withSms: injected });
  sweep.run();
  eq(sweep.fakeWindow.sms, injected, 'pre-existing window.sms is not replaced');
  eq(sweep.state.calls.length, 0, 'no IPC when window.sms already exists');
  eq(sweep.state.listeners.length, 0, 'no event listeners when window.sms already exists');

  // 2) plain browser / no Tauri global: define nothing
  const plain = makeEnv({ withoutTauri: true });
  plain.run();
  ok(!plain.fakeWindow.sms, 'window.sms is not created outside Tauri');
  eq(plain.state.calls.length, 0, 'no IPC outside Tauri');

  // 3) loaded twice: second load is a no-op
  const twice = makeEnv();
  twice.run();
  const firstBridge = twice.fakeWindow.sms;
  twice.run();
  ok(twice.fakeWindow.sms === firstBridge, 'double load keeps the first bridge');
  eq(twice.state.calls.length, 0, 'double load does not call IPC');
}

async function checkTitlebar() {
  const env = makeEnv();
  env.run();
  const down = env.titlebarHandlers.mousedown || [];
  const dbl = env.titlebarHandlers.dblclick || [];
  ok(down.length === 1, 'mousedown drag handler registered on .titlebar', `got ${down.length}`);
  ok(dbl.length === 1, 'dblclick handler registered on .titlebar', `got ${dbl.length}`);

  const fakeTarget = (closestResult) => ({
    closest() {
      return closestResult;
    },
  });

  // left mousedown on the drag strip starts the native drag
  down[0]({ button: 0, detail: 1, target: fakeTarget(null) });
  eq(env.state.drags.length, 1, 'mousedown on titlebar calls startDragging()');

  // buttons / win-actions / brand / theme-btn stay clickable
  down[0]({ button: 0, detail: 1, target: fakeTarget({}) });
  eq(env.state.drags.length, 1, 'mousedown on chrome button does not startDragging()');

  // right click does not drag
  down[0]({ button: 2, detail: 1, target: fakeTarget(null) });
  eq(env.state.drags.length, 1, 'non-left mousedown does not startDragging()');

  // second click of a double click maximises instead of dragging
  env.state.calls.length = 0;
  down[0]({ button: 0, detail: 2, target: fakeTarget(null) });
  eq(env.state.drags.length, 1, 'mousedown detail=2 does not startDragging()');
  eq(env.state.calls.length, 1, 'mousedown detail=2 invokes window:toggle-maximize');
  eq(env.state.calls[0] && env.state.calls[0].channel, rustCommand('window:toggle-maximize'), 'mousedown detail=2 command');

  // the dblclick that follows the same gesture must not toggle a second time
  dbl[0]({ target: fakeTarget(null) });
  eq(env.state.calls.length, 1, 'dblclick after mousedown#2 does not toggle twice');

  // and app.js's own titlebar dblclick handler calls sms.win.toggleMaximize();
  // that duplicate call must be absorbed as well
  await env.fakeWindow.sms.win.toggleMaximize();
  eq(env.state.calls.length, 1, 'app.js duplicate dblclick toggle is suppressed');

  // standalone dblclick (fresh page) toggles exactly once
  const fresh = makeEnv();
  fresh.run();
  const freshDbl = fresh.titlebarHandlers.dblclick || [];
  freshDbl[0]({ target: fakeTarget(null) });
  eq(fresh.state.calls.length, 1, 'dblclick invokes window:toggle-maximize');
  eq(fresh.state.calls[0] && fresh.state.calls[0].channel, rustCommand('window:toggle-maximize'), 'dblclick command');
}

async function main() {
  if (!fs.existsSync(SHIM_PATH)) {
    console.error(`[check-shim] missing shim: ${SHIM_PATH}`);
    process.exit(1);
  }

  await checkContract();
  await checkEvents();
  await checkRejectionNormalisation();
  await checkGuards();
  await checkTitlebar();

  if (failures.length) {
    console.error(`[check-shim] FAIL — ${failures.length} assertion(s)`);
    for (const f of failures) {
      console.error(`\n  FAIL ${f.label}`);
      if (f.detail) console.error(f.detail.split('\n').map((line) => `    ${line}`).join('\n'));
    }
    process.exit(1);
  }

  console.log(`[check-shim] PASS — ${METHODS.length} IPC channels, ${EVENTS.length} events, rejection normalisation, guards, titlebar drag`);
  process.exit(0);
}

main().catch((err) => {
  console.error('[check-shim] crashed:', err && err.stack ? err.stack : err);
  process.exit(1);
});

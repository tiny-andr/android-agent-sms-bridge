/*
 * tauri.js — window.sms bridge for the Tauri 2 client.
 *
 * Drop-in replacement for the Electron preload (windows/preload.js): app.js
 * talks to the backend only through this object, so the shape and behaviour
 * must stay identical. No modules, no build step, ES5-compatible browser code.
 *
 * Loaded before app.js by ui/index.html. tauri.conf.json enables
 * app.withGlobalTauri, so the IPC API is window.__TAURI__.core.invoke and the
 * event API is window.__TAURI__.event.listen.
 */
(function () {
  'use strict';

  // Never clobber an existing bridge: the Electron-based i18n sweep injects its
  // own window.sms before the page scripts run, and loading tauri.js twice must
  // be harmless as well.
  if (typeof window === 'undefined' || window.sms) return;

  var tauri = window.__TAURI__;
  if (!tauri || !tauri.core || typeof tauri.core.invoke !== 'function') return; // not running inside Tauri

  /* ------------------------------------------------------------ plumbing ── */

  // Tauri rejects app commands with whatever the Rust Err(_) serialises to —
  // usually a bare string. app.js reads err.message, so normalise everything.
  function toMessage(err) {
    if (typeof err === 'string') return err;
    if (err && typeof err.message === 'string' && err.message) return err.message;
    try {
      return String(err);
    } catch (_) {
      return 'unknown error';
    }
  }

  function call(channel, args) {
    var pending;
    try {
      // The renderer-facing channel names are the Electron IPC names and stay
      // exactly as they were ("state:get", "window:toggle-maximize", …). Tauri
      // derives the command name from the Rust function identifier, so the
      // mapping is the mechanical `:`/`-` → `_` translation, applied here and
      // asserted by scripts/check-shim.cjs. One channel ↔ one command, no
      // merging and no renames on the renderer side.
      pending = tauri.core.invoke(rustCommand(channel), args);
    } catch (err) {
      return Promise.reject(new Error(toMessage(err)));
    }
    return Promise.resolve(pending).then(undefined, function (err) {
      throw new Error(toMessage(err));
    });
  }

  function rustCommand(channel) {
    return String(channel).replace(/[:-]/g, '_');
  }

  function noop() {}

  function subscribe(channel, cb) {
    var unlisten = null;
    var disposed = false;
    try {
      var pending = tauri.event.listen(channel, function (evt) {
        cb(evt && evt.payload);
      });
      if (pending && typeof pending.then === 'function') {
        pending.then(function (fn) {
          if (disposed) {
            try { fn(); } catch (_) {}
            return;
          }
          unlisten = fn;
        }, noop);
      }
    } catch (_) {}
    // app.js ignores this return value today, but preload returned a real
    // unlisten function and callers may rely on it.
    return function () {
      disposed = true;
      if (typeof unlisten !== 'function') return;
      try {
        return unlisten();
      } catch (_) {}
    };
  }

  /* ------------------------------------------------------- window handling ── */

  function currentWindow() {
    try {
      var w = tauri.window;
      if (w && typeof w.getCurrentWindow === 'function') return w.getCurrentWindow();
    } catch (_) {}
    return null;
  }

  // app.js already toggles maximise on its own titlebar dblclick handler (it
  // calls window.sms.win.toggleMaximize). The shim also handles the titlebar
  // gesture for parity with the Electron drag region; these two flags make a
  // single double click produce exactly one toggle, never two.
  var suppressToggleOnce = false;
  var lastToggleAt = 0;

  function requestMaximizeToggle() {
    var now = Date.now();
    // mousedown#2 and the following dblclick belong to the same gesture
    if (now - lastToggleAt < 400) return;
    lastToggleAt = now;
    suppressToggleOnce = true;
    setTimeout(function () { suppressToggleOnce = false; }, 600);
    call('window:toggle-maximize');
  }

  function bindTitlebar() {
    var doc = typeof document !== 'undefined' ? document : null;
    var bar = doc && doc.querySelector ? doc.querySelector('.titlebar') : null;
    if (!bar || typeof bar.addEventListener !== 'function') return;

    function inExcludedChrome(target) {
      if (!target || typeof target.closest !== 'function') return false;
      return !!target.closest('button, .win-actions, .brand, .theme-btn');
    }

    // -webkit-app-region: drag is inert in WebView2, so start the native move
    // loop ourselves. Buttons / brand / theme button stay clickable.
    bar.addEventListener('mousedown', function (evt) {
      if (!evt || evt.button !== 0) return; // left button only
      if (inExcludedChrome(evt.target)) return;
      // second click of a titlebar double click: maximise, do not drag
      if (evt.detail === 2) {
        requestMaximizeToggle();
        return;
      }
      if (evt.detail > 2) return;
      var win = currentWindow();
      if (!win || typeof win.startDragging !== 'function') return;
      try {
        var pending = win.startDragging();
        if (pending && typeof pending.then === 'function') pending.then(undefined, noop);
      } catch (_) {}
    });

    bar.addEventListener('dblclick', function (evt) {
      if (inExcludedChrome(evt.target)) return;
      requestMaximizeToggle();
    });
  }

  /* --------------------------------------------------------------- bridge ── */

  function pair(payload) {
    payload = payload || {};
    return call('phone:pair', {
      name: payload.name,
      url: payload.url,
      code: payload.code,
      phoneId: payload.phoneId == null ? null : payload.phoneId,
    });
  }

  function importPhone(payload) {
    payload = payload || {};
    return call('phone:import', {
      name: payload.name,
      url: payload.url,
      clientId: payload.clientId,
      token: payload.token,
    });
  }

  window.sms = {
    getState: function () { return call('state:get'); },

    pair: pair,
    importPhone: importPhone,
    probe: function (url) { return call('phone:probe', { url: url }); },
    removePhone: function (id) { return call('phone:remove', { id: id }); },
    renamePhone: function (id, name) { return call('phone:rename', { id: id, name: name }); },
    phoneHealth: function (id) { return call('phone:health', { id: id }); },

    syncOne: function (id) { return call('sync:one', { id: id }); },
    syncAll: function () { return call('sync:all'); },
    updateCheck: function () { return call('update:check'); },
    updateApply: function () { return call('update:apply'); },
    setSettings: function (patch) { return call('settings:set', { patch: patch }); },
    clearMessages: function (phoneId) { return call('messages:clear', { phoneId: phoneId }); },
    markSeen: function (address) { return call('messages:seen', { address: address }); },
    exportMessages: function (phoneId) { return call('messages:export', { phoneId: phoneId }); },
    openDataDir: function () { return call('app:open-data-dir'); },
    openExternal: function (url) { return call('app:open-external', { url: url }); },
    revealConfig: function () { return call('app:reveal-config'); },

    win: {
      minimize: function () { return call('window:minimize'); },
      toggleMaximize: function () {
        if (suppressToggleOnce) {
          suppressToggleOnce = false;
          return Promise.resolve();
        }
        lastToggleAt = Date.now();
        return call('window:toggle-maximize');
      },
      close: function () { return call('window:close'); },
      isMaximized: function () { return call('window:is-maximized'); },
    },

    onStateChanged: function (cb) { return subscribe('state:changed', cb); },
    onSyncProgress: function (cb) { return subscribe('sync:progress', cb); },
    onSyncStarted: function (cb) { return subscribe('sync:started', cb); },
    onSyncError: function (cb) { return subscribe('sync:error', cb); },
    onUpdateProgress: function (cb) { return subscribe('update:progress', cb); },
    onSettingsUpdate: function (cb) { return subscribe('settings:update', cb); },
    onMaximized: function (cb) { return subscribe('window:maximized', cb); },
  };

  bindTitlebar();
})();

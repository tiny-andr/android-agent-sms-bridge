'use strict';
const { contextBridge, ipcRenderer } = require('electron');

const listener = (channel) => (cb) => {
  const handler = (_e, payload) => cb(payload);
  ipcRenderer.on(channel, handler);
  return () => ipcRenderer.removeListener(channel, handler);
};

contextBridge.exposeInMainWorld('sms', {
  getState: () => ipcRenderer.invoke('state:get'),

  pair: (payload) => ipcRenderer.invoke('phone:pair', payload),
  importPhone: (payload) => ipcRenderer.invoke('phone:import', payload),
  probe: (url) => ipcRenderer.invoke('phone:probe', url),
  removePhone: (id) => ipcRenderer.invoke('phone:remove', id),
  renamePhone: (id, name) => ipcRenderer.invoke('phone:rename', { id, name }),
  phoneHealth: (id) => ipcRenderer.invoke('phone:health', id),

  syncOne: (id) => ipcRenderer.invoke('sync:one', { id }),
  syncAll: () => ipcRenderer.invoke('sync:all', {}),
  setSettings: (patch) => ipcRenderer.invoke('settings:set', patch),
  clearMessages: (phoneId) => ipcRenderer.invoke('messages:clear', phoneId),
  markSeen: (address) => ipcRenderer.invoke('messages:seen', address),
  exportMessages: (phoneId) => ipcRenderer.invoke('messages:export', { phoneId }),
  openDataDir: () => ipcRenderer.invoke('app:open-data-dir'),
  openExternal: (url) => ipcRenderer.invoke('app:open-external', url),
  revealConfig: () => ipcRenderer.invoke('app:reveal-config'),

  win: {
    minimize: () => ipcRenderer.invoke('window:minimize'),
    toggleMaximize: () => ipcRenderer.invoke('window:toggle-maximize'),
    close: () => ipcRenderer.invoke('window:close'),
    isMaximized: () => ipcRenderer.invoke('window:is-maximized'),
  },

  onStateChanged: listener('state:changed'),
  onSyncProgress: listener('sync:progress'),
  onSyncStarted: listener('sync:started'),
  onSyncError: listener('sync:error'),
  onSettingsUpdate: listener('settings:update'),
  onMaximized: listener('window:maximized'),
});

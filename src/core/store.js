'use strict';
/**
 * Local persistence: config (phones + settings) and the cached SMS database.
 *
 * The pairing token is a bearer secret — on Windows we seal it with DPAPI via
 * Electron's safeStorage so a copied config file is useless on another machine.
 * JSON is written atomically (tmp + rename) to survive a crash mid-write.
 */

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const CONFIG_VERSION = 1;

const DEFAULT_SETTINGS = {
  autoStart: false,        // 开机自启
  startHidden: false,      // 启动后最小化到托盘
  syncIntervalMin: 15,     // 0 = 关闭定时同步
  syncOnStart: true,       // 启动后自动同步一次
  notifyNew: true,         // 新短信桌面通知
  pullDays: 30,            // 每次从手机拉取的时间范围（天）
  keepDays: 0,             // 本地保留天数，0 = 永久
  closeToTray: true,       // 关闭窗口时最小化到托盘
  theme: 'dark',           // 'dark' | 'light'
  uiScale: 1.1,            // 0.85 – 1.5, renderer zoom factor (1.1 = comfortable default)
  lang: 'auto',            // 'auto' | 'zh' | 'en' | 'ja'
};

function emptyDb() {
  return { version: 1, messages: {} };
}

class Store {
  constructor({ dir, safeStorage = null }) {
    this.dir = dir;
    this.safe = safeStorage;
    this.configPath = path.join(dir, 'config.json');
    this.dbPath = path.join(dir, 'messages.json');
    this.config = null;
    this.db = null;
    this._saveTimer = null;
  }

  // ------------------------------------------------------------------ crypto

  sealToken(token) {
    if (!token) return { tokenEnc: '', tokenRaw: '' };
    try {
      if (this.safe && this.safe.isEncryptionAvailable()) {
        return { tokenEnc: this.safe.encryptString(token).toString('base64'), tokenRaw: '' };
      }
    } catch { /* fall through to plaintext */ }
    return { tokenEnc: '', tokenRaw: token };
  }

  openToken(phone) {
    if (phone.tokenEnc) {
      try {
        return this.safe.decryptString(Buffer.from(phone.tokenEnc, 'base64'));
      } catch {
        return '';
      }
    }
    return phone.tokenRaw || '';
  }

  // ------------------------------------------------------------------- load

  load() {
    try {
      this.config = JSON.parse(fs.readFileSync(this.configPath, 'utf8'));
    } catch {
      this.config = { version: CONFIG_VERSION, settings: {}, phones: [] };
    }
    this.config.settings = { ...DEFAULT_SETTINGS, ...(this.config.settings || {}) };
    this.config.phones = Array.isArray(this.config.phones) ? this.config.phones : [];
    this.config.version = CONFIG_VERSION;

    try {
      this.db = JSON.parse(fs.readFileSync(this.dbPath, 'utf8'));
    } catch {
      this.db = emptyDb();
    }
    if (!this.db || typeof this.db !== 'object') this.db = emptyDb();
    if (!this.db.messages) this.db.messages = {};

    // migrate plaintext tokens to DPAPI when possible
    let migrated = false;
    for (const p of this.config.phones) {
      // 1.0.1+ serves TLS only; a stored http URL would fail with a protocol error
      if (typeof p.url === 'string' && /^http:\/\//i.test(p.url)) {
        p.url = p.url.replace(/^http:\/\//i, 'https://');
        migrated = true;
      }
      if (typeof p.certPem !== 'string') p.certPem = p.certPem || '';
      if (typeof p.certFp !== 'string') p.certFp = p.certFp || '';
      if (p.tokenRaw && this.safe && this.safe.isEncryptionAvailable()) {
        const sealed = this.sealToken(p.tokenRaw);
        if (sealed.tokenEnc) {
          p.tokenEnc = sealed.tokenEnc;
          p.tokenRaw = '';
          migrated = true;
        }
      }
    }
    if (migrated) this.saveConfig();
    return this;
  }

  // ------------------------------------------------------------------- save

  saveConfig() {
    writeAtomic(this.configPath, JSON.stringify(this.config, null, 2));
  }

  saveDbSoon() {
    if (this._saveTimer) return;
    this._saveTimer = setTimeout(() => {
      this._saveTimer = null;
      this.flushDb();
    }, 800);
  }

  flushDb() {
    if (this._saveTimer) {
      clearTimeout(this._saveTimer);
      this._saveTimer = null;
    }
    writeAtomic(this.dbPath, JSON.stringify(this.db));
  }

  // ---------------------------------------------------------------- settings

  get settings() { return this.config.settings; }

  updateSettings(patch) {
    Object.assign(this.config.settings, patch || {});
    this.saveConfig();
    return this.config.settings;
  }

  // ------------------------------------------------------------------ phones

  listPhones() {
    return this.config.phones.map((p) => this.publicPhone(p));
  }

  publicPhone(p) {
    return {
      id: p.id,
      name: p.name,
      url: p.url,
      clientName: p.clientName || '',
      createdAt: p.createdAt || 0,
      lastSyncAt: p.lastSyncAt || 0,
      lastError: p.lastError || '',
      authError: Boolean(p.authError),
      health: p.health || null,
      hasToken: Boolean(p.tokenEnc || p.tokenRaw),
      // fingerprint only — the PEM stays in the config, the UI never needs it
      certFp: p.certFp || '',
    };
  }

  getPhone(id) { return this.config.phones.find((p) => p.id === id) || null; }

  addPhone({ name, url, clientId, token, clientName, certPem, certFp }) {
    const phone = {
      id: crypto.randomUUID(),
      name: name || '我的手机',
      url,
      clientId,
      clientName: clientName || '',
      createdAt: Date.now(),
      lastSyncAt: 0,
      lastSeq: 0,
      lastSmsId: 0,
      certPem: certPem || '',
      certFp: certFp || '',
      ...this.sealToken(token),
    };
    this.config.phones.push(phone);
    this.saveConfig();
    return this.publicPhone(phone);
  }

  updatePhone(id, patch) {
    const p = this.getPhone(id);
    if (!p) return null;
    Object.assign(p, patch);
    this.saveConfig();
    return this.publicPhone(p);
  }

  removePhone(id) {
    this.config.phones = this.config.phones.filter((p) => p.id !== id);
    delete this.db.messages[id];
    this.saveConfig();
    this.flushDb();
  }

  tokenOf(id) {
    const p = this.getPhone(id);
    return p ? this.openToken(p) : '';
  }

  // ---------------------------------------------------------------- messages

  upsertMessages(phoneId, rows) {
    const bucket = this.db.messages[phoneId] || (this.db.messages[phoneId] = []);
    const index = new Map(bucket.map((m, i) => [m.id, i]));
    let added = 0;
    let updated = 0;
    for (const row of rows) {
      const i = index.get(row.id);
      if (i === undefined) {
        index.set(row.id, bucket.length);
        bucket.push(row);
        added++;
      } else {
        const prev = bucket[i];
        if (prev.body !== row.body || prev.read !== row.read
            || prev.simNumber !== row.simNumber || prev.carrier !== row.carrier
            || prev.slot !== row.slot || prev.subId !== row.subId) {
          bucket[i] = {
            ...prev,
            body: row.body,
            read: row.read,
            simNumber: row.simNumber,
            slot: row.slot,
            subId: row.subId,
            carrier: row.carrier,
          };
          updated++;
        }
      }
    }
    bucket.sort((a, b) => b.date - a.date);
    this.saveDbSoon();
    return { added, updated, total: bucket.length };
  }

  messagesOf(phoneId) {
    return this.db.messages[phoneId] || [];
  }

  /** Local read marker: the phone's `read` column never changes because we
   *  only ever read. `seen` lets the UI dismiss unread badges without a write. */
  markSeen(address) {
    let n = 0;
    for (const list of Object.values(this.db.messages)) {
      for (const m of list) {
        if (m.address === address && !m.seen) { m.seen = true; n++; }
      }
    }
    if (n) this.flushDb();
    return n;
  }

  allMessages() {
    const out = [];
    for (const [phoneId, list] of Object.entries(this.db.messages)) {
      for (const m of list) out.push({ ...m, phoneId });
    }
    out.sort((a, b) => b.date - a.date);
    return out;
  }

  stats() {
    let count = 0;
    for (const list of Object.values(this.db.messages)) count += list.length;
    return { total: count, phones: Object.keys(this.db.messages).length };
  }

  prune() {
    const keepDays = Number(this.settings.keepDays) || 0;
    if (keepDays <= 0) return 0;
    const cutoff = Date.now() - keepDays * 86400000;
    let removed = 0;
    for (const [phoneId, list] of Object.entries(this.db.messages)) {
      const kept = list.filter((m) => m.date >= cutoff);
      removed += list.length - kept.length;
      this.db.messages[phoneId] = kept;
    }
    if (removed) this.flushDb();
    return removed;
  }

  clearMessages(phoneId) {
    if (phoneId) delete this.db.messages[phoneId];
    else this.db.messages = {};
    this.flushDb();
  }
}

function writeAtomic(file, data) {
  const tmp = file + '.tmp';
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(tmp, data, 'utf8');
  fs.renameSync(tmp, file);
}

module.exports = { Store, DEFAULT_SETTINGS };

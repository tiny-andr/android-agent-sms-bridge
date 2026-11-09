'use strict';

/* ============================================================================
   Windows SMS Bridge — renderer
   ========================================================================= */

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

const { t, localizeText, localizeDom, relocalizeDom, setLang, resolveLang } = window.I18N;

// provisional language until the stored setting arrives, so a zh system never
// flashes translated chrome
setLang(resolveLang('auto', navigator.language));

const state = {
  phones: [],
  messages: [],
  settings: {},
  stats: { total: 0 },
  sync: {},
  app: {},
  view: { mode: 'all', phoneId: null, query: '', sim: null },
  selected: null,
  syncing: false,
  loading: true,
};

/* ------------------------------------------------------------------ utils ── */

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => (
  { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]
));

const pad2 = (n) => String(n).padStart(2, '0');

/** Strip Electron's IPC wrapper so users see the real reason. */
function errText(err) {
  let m = (err && err.message) || String(err);
  m = m.replace(/^Error invoking remote method '[^']*':\s*/, '');
  m = m.replace(/^(BridgeError|Error|TypeError):\s*/, '');
  return m;
}

function dayKey(ms) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

function fmtListTime(ms) {
  if (!ms) return '';
  const d = new Date(ms);
  const now = new Date();
  const sameDay = dayKey(ms) === dayKey(now.getTime());
  const yest = new Date(now.getTime() - 86400000);
  if (sameDay) return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  if (dayKey(ms) === dayKey(yest.getTime())) return '昨天';
  if (d.getFullYear() === now.getFullYear()) return `${d.getMonth() + 1}/${d.getDate()}`;
  return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`;
}

function fmtClock(ms) {
  const d = new Date(ms);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function fmtDayLabel(ms) {
  const d = new Date(ms);
  const now = new Date();
  if (dayKey(ms) === dayKey(now.getTime())) return '今天';
  if (dayKey(ms) === dayKey(now.getTime() - 86400000)) return '昨天';
  const sameYear = d.getFullYear() === now.getFullYear();
  return sameYear
    ? t('{m} 月 {d} 日', { m: d.getMonth() + 1, d: d.getDate() })
    : t('{y} 年 {m} 月 {d} 日', { y: d.getFullYear(), m: d.getMonth() + 1, d: d.getDate() });
}

function fmtAgo(ms) {
  if (!ms) return '';
  const diff = Date.now() - ms;
  if (diff < 60000) return '刚刚';
  if (diff < 3600000) return t('{n} 分钟前', { n: Math.round(diff / 60000) });
  if (diff < 86400000) return t('{n} 小时前', { n: Math.round(diff / 3600000) });
  if (diff < 2592000000) return t('{n} 天前', { n: Math.round(diff / 86400000) });
  const d = new Date(ms);
  return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`;
}

function fmtRelative(ms) {
  if (!ms) return t('从未同步');
  const diff = Date.now() - ms;
  if (diff < 45000) return t('刚刚同步');
  if (diff < 3600000) return t('{n} 分钟前同步', { n: Math.round(diff / 60000) });
  if (diff < 86400000) return t('{n} 小时前同步', { n: Math.round(diff / 3600000) });
  return t('{n} 天前同步', { n: Math.round(diff / 86400000) });
}

function fmtBytes(n) { return n > 1024 ? `${(n / 1024).toFixed(1)} KB` : `${n} B`; }

function shortAddress(addr) {
  const a = String(addr || '').trim();
  if (/^\+?86\d{11}$/.test(a.replace(/\s/g, ''))) return a.replace(/^\+?86/, '');
  return a || '未知号码';
}

/** Monochrome glass tile: the fill is the shared 8% tint, the ink is the text colour. */
function avatarStyle() {
  return 'background: rgb(var(--tint) / 0.08);';
}

/* ------------------------------------------------------------------ sim ─── */

/** Digits of a SIM number with the +86 country code removed. */
const simDigits = (num) => String(num || '').replace(/\D/g, '').replace(/^86(?=1\d{10}$)/, '');
const simTail = (num) => simDigits(num).slice(-4);

/** 19973712132 → "199 7371 2132". */
function fmtPhone(num) {
  const d = simDigits(num);
  return d.length === 11 ? `${d.slice(0, 3)} ${d.slice(3, 7)} ${d.slice(7)}` : d;
}

/**
 * Stable grouping key for the receiving SIM: the physical slot, because rows
 * cached before 1.1.0 carry the slot but not the number. Keying by number would
 * split one card into "slot:0" and "num:..." groups after an upgrade.
 */
function simKeyOf(m) {
  if (m && typeof m.slot === 'number' && m.slot >= 0) return `slot:${m.slot}`;
  if (m && m.simNumber) return `num:${simDigits(m.simNumber)}`;
  return '';
}

/** The live number of a slot, taken from the phone's health snapshot. */
function slotNumber(phoneId, slot) {
  const p = state.phones.find((x) => x.id === phoneId);
  const sims = (p && p.health && Array.isArray(p.health.sims)) ? p.health.sims : [];
  const hit = sims.find((s) => s.slot === slot);
  return hit && hit.number ? hit.number : '';
}

/** "卡1 · 2132" — slot plus the last four digits, or just "卡1" when unknown. */
function simLabel(slot, number) {
  const hasSlot = typeof slot === 'number' && slot >= 0;
  const tail = simTail(number);
  if (hasSlot && tail) return t('卡{n} · {tail}', { n: slot + 1, tail });
  if (hasSlot) return t('卡{n}', { n: slot + 1 });
  return tail ? t('尾号 {tail}', { tail }) : '';
}

function simLabelOf(m) {
  const slot = m && m.slot;
  // Rows cached before 1.1.0 have no simNumber, so fall back to the live card.
  const number = (m && m.simNumber) || slotNumber(m && m.phoneId, slot);
  return simLabel(slot, number);
}

/** Sidebar label: the number itself when known, otherwise the slot. */
function simOptionLabel(slot, number) {
  return fmtPhone(number) || simLabel(slot, '');
}

/** Every card of the selected phone, merged from its health snapshot and cache. */
function simOptions() {
  // A single paired phone IS the scope even when the "all devices" chip is
  // active, otherwise its SIM filter would never appear.
  const scopeId = state.view.phoneId || (state.phones.length === 1 ? state.phones[0].id : null);
  const phone = state.phones.find((p) => p.id === scopeId);
  const healthSims = (phone && phone.health && Array.isArray(phone.health.sims)) ? phone.health.sims : [];
  const opts = new Map();
  for (const s of healthSims) {
    const key = `slot:${s.slot}`;
    opts.set(key, {
      key, slot: s.slot, number: s.number || '', label: simOptionLabel(s.slot, s.number), count: 0,
    });
  }
  const mode = state.view.mode;
  for (const m of state.messages) {
    if (state.view.phoneId && m.phoneId !== state.view.phoneId) continue;
    const key = simKeyOf(m);
    if (!key) continue;
    let o = opts.get(key);
    if (!o) {
      const number = m.simNumber || slotNumber(m.phoneId, m.slot);
      o = { key, slot: m.slot, number, label: simOptionLabel(m.slot, number), count: 0 };
      opts.set(key, o);
    } else if (!o.number && m.simNumber) {
      o.number = m.simNumber;
      o.label = simOptionLabel(o.slot, o.number);
    }
    if (mode === 'inbox' && m.type !== 'inbox') continue;
    if (mode === 'sent' && m.type !== 'sent') continue;
    if (mode === 'unread' && !(m.type === 'inbox' && !m.read && !m.seen)) continue;
    if (m.type !== 'draft') o.count++;
  }
  return [...opts.values()].filter((o) => o.slot >= 0).sort((a, b) => a.slot - b.slot);
}


function initials(addr) {
  const a = String(addr || '').trim();
  if (!a) return '?';
  const digits = a.replace(/[^\d]/g, '');
  if (digits.length >= 5) return digits.slice(-2);
  if (/^[+\d]/.test(a)) return a.slice(0, 2);
  return a.slice(0, 2).toUpperCase();
}

/** Repos shown in the About sheet. */
const REPO_PHONE = 'https://github.com/tiny-andr/android-agent-sms-bridge';
const REPO_SELF = 'https://github.com/tiny-andr/win-sms-bridge';

const THEMES = ['dark', 'light', 'paper'];
const THEME_NEXT = { dark: 'light', light: 'paper', paper: 'dark' };
const THEME_LABEL = { dark: '深色', light: '冷白', paper: '米白' };
const ICON_SUN = '<circle cx="12" cy="12" r="4.2"/><path d="M12 2.6v2.2M12 19.2v2.2M2.6 12h2.2M19.2 12h2.2M5.4 5.4l1.6 1.6M17 17l1.6 1.6M18.6 5.4L17 7M7 17l-1.6 1.6"/>';
const ICON_MOON = '<path d="M20.2 14.4A8.4 8.4 0 0 1 9.6 3.8 8.4 8.4 0 1 0 20.2 14.4Z"/>';

function brandMark() {
  return `<svg viewBox="0 0 18 18" fill="none" stroke="#fff" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
    <path d="M3 5.2A2.2 2.2 0 0 1 5.2 3h7.6A2.2 2.2 0 0 1 15 5.2v5.2a2.2 2.2 0 0 1-2.2 2.2H8.4L5 15.4v-2.9A2.2 2.2 0 0 1 3 10.4z"/>
  </svg>`;
}

const SCALE_MIN = 0.85;
const SCALE_MAX = 1.5;
const clampScale = (v) => Math.min(SCALE_MAX, Math.max(SCALE_MIN, Math.round(v * 100) / 100));
const scaleOf = (settings) => clampScale(Number((settings && settings.uiScale) || 1));

function applyTheme(theme, { persist = false } = {}) {
  const next = THEMES.includes(theme) ? theme : 'dark';
  document.documentElement.dataset.theme = next;
  const icon = $('#themeIcon');
  if (icon) icon.innerHTML = next === 'dark' ? ICON_SUN : ICON_MOON;
  const btn = $('#btnTheme');
  if (btn) btn.title = t('切换到' + THEME_LABEL[THEME_NEXT[next]] + '主题');
  if (persist) {
    state.settings = { ...state.settings, theme: next };
    window.sms.setSettings({ theme: next }).catch(() => {});
  }
  return next;
}

const ICONS = {
  all: '<svg viewBox="0 0 18 18" class="ic"><rect x="2.4" y="2.4" width="5.6" height="5.6" rx="1.8"/><rect x="10" y="2.4" width="5.6" height="5.6" rx="1.8"/><rect x="2.4" y="10" width="5.6" height="5.6" rx="1.8"/><rect x="10" y="10" width="5.6" height="5.6" rx="1.8"/></svg>',
  inbox: '<svg viewBox="0 0 18 18" class="ic"><path d="M2.6 9.6h3.2l1.1 2h4.2l1.1-2h3.2"/><path d="M4.4 3.4h9.2a1.8 1.8 0 0 1 1.8 1.8v7.6a1.8 1.8 0 0 1-1.8 1.8H4.4a1.8 1.8 0 0 1-1.8-1.8V5.2a1.8 1.8 0 0 1 1.8-1.8Z"/></svg>',
  sent: '<svg viewBox="0 0 18 18" class="ic"><path d="M15.6 2.4L8.2 15.4 6.7 9.6 2.4 7.5z"/><path d="M6.7 9.6l8.9-7.2"/></svg>',
  key: '<svg viewBox="0 0 18 18" class="ic"><rect x="4" y="8" width="10" height="7.4" rx="2.2"/><path d="M6.4 8V6.2a2.6 2.6 0 0 1 5.2 0V8"/><path d="M9 11v1.8"/></svg>',
  unread: '<svg viewBox="0 0 18 18" class="ic"><circle cx="9" cy="9" r="6.6"/><circle cx="9" cy="9" r="2.4" fill="currentColor" stroke="none"/></svg>',
  sync: '<svg viewBox="0 0 18 18" class="ic"><path d="M14.8 7.4A6 6 0 0 0 4.2 5.8M3.2 10.6a6 6 0 0 0 10.6 1.6"/><path d="M15 3.6v3.8h-3.8M3 14.4v-3.8h3.8"/></svg>',
  trash: '<svg viewBox="0 0 18 18" class="ic"><path d="M3.6 5.2h10.8M7 5.2V3.8h4v1.4M4.8 5.2l.7 9.1a1.4 1.4 0 0 0 1.4 1.3h4.2a1.4 1.4 0 0 0 1.4-1.3l.7-9.1"/></svg>',
  sim: '<svg viewBox="0 0 18 18" class="ic"><rect x="4.6" y="2.4" width="8.8" height="13.2" rx="1.8"/><path d="M7 5.8h4v3.2H7z"/></svg>',
};

/* --------------------------------------------------------------- derive ─── */

function messageMatchesView(m) {
  if (state.view.phoneId && m.phoneId !== state.view.phoneId) return false;
  if (state.view.sim && simKeyOf(m) !== state.view.sim) return false;
  if (state.view.mode === 'inbox' && m.type !== 'inbox') return false;
  if (state.view.mode === 'sent' && m.type !== 'sent') return false;
  if (state.view.mode === 'unread' && !(m.type === 'inbox' && !m.read && !m.seen)) return false;
  if (m.type === 'draft') return false;
  const q = state.view.query.trim().toLowerCase();
  if (q && !(String(m.address).toLowerCase().includes(q) || String(m.body).toLowerCase().includes(q))) return false;
  return true;
}

function buildConversations() {
  const map = new Map();
  for (const m of state.messages) {
    if (!messageMatchesView(m)) continue;
    const key = m.address || '未知号码';
    let conv = map.get(key);
    if (!conv) {
      conv = { key, address: key, messages: [], unread: 0, phones: new Set() };
      map.set(key, conv);
    }
    conv.messages.push(m);
    conv.phones.add(m.phoneId);
    if (m.type === 'inbox' && !m.read && !m.seen) conv.unread++;
  }
  const list = Array.from(map.values());
  for (const c of list) {
    c.messages.sort((a, b) => a.date - b.date);
    c.last = c.messages[c.messages.length - 1];
    c.count = c.messages.length;
  }
  list.sort((a, b) => b.last.date - a.last.date);
  return list;
}

function countBy(mode, ignoreSim = false) {
  let n = 0;
  for (const m of state.messages) {
    if (m.type === 'draft') continue;
    if (state.view.phoneId && m.phoneId !== state.view.phoneId) continue;
    if (!ignoreSim && state.view.sim && simKeyOf(m) !== state.view.sim) continue;
    if (mode === 'inbox' && m.type !== 'inbox') continue;
    if (mode === 'sent' && m.type !== 'sent') continue;
    if (mode === 'unread' && !(m.type === 'inbox' && !m.read && !m.seen)) continue;
    n++;
  }
  return n;
}

/* --------------------------------------------------------------- render ── */

function renderNav() {
  const items = [
    { id: 'all', label: '全部短信', icon: ICONS.all, count: countBy('all') },
    { id: 'inbox', label: '收件箱', icon: ICONS.inbox, count: countBy('inbox') },
    { id: 'sent', label: '已发送', icon: ICONS.sent, count: countBy('sent') },
    { id: 'unread', label: '未读', icon: ICONS.unread, count: countBy('unread') },
  ];
  $('#navFilters').innerHTML = items.map((it) => `
    <button class="nav-item ${state.view.mode === it.id ? 'on' : ''}" data-mode="${it.id}">
      ${it.icon}<span>${it.label}</span>
      ${it.count ? `<span class="badge">${it.count > 999 ? '999+' : it.count}</span>` : ''}
    </button>`).join('');

  // SIM filter: only meaningful once a single phone is selected and it has 2+
  // cards, otherwise every message belongs to the same card anyway.
  const simSection = $('#simSection');
  const simNav = $('#navSims');
  if (!simSection || !simNav) return;
  const sims = (state.view.phoneId || state.phones.length === 1) ? simOptions() : [];
  if (sims.length < 2) {
    simSection.hidden = true;
    if (state.view.sim) state.view.sim = null;
    return;
  }
  if (state.view.sim && !sims.some((s) => s.key === state.view.sim)) state.view.sim = null;
  simSection.hidden = false;
  const rows = [{ key: '', label: t('全部卡'), count: countBy(state.view.mode, true) }, ...sims];
  simNav.innerHTML = rows.map((s) => `
    <button class="nav-item ${(state.view.sim || '') === s.key ? 'on' : ''}" data-sim="${esc(s.key)}">
      ${ICONS.sim}<span>${esc(s.label)}</span>
      ${s.count ? `<span class="badge">${s.count > 999 ? '999+' : s.count}</span>` : ''}
    </button>`).join('');
}

function deviceStateDot(p) {
  const h = p.health;
  const fresh = h && h.fetchedAt && Date.now() - h.fetchedAt < 180000;
  return !h ? 'offline' : fresh ? 'online' : 'warn';
}

function renderDevices() {
  const wrap = $('#deviceChips');
  if (!wrap) return;
  const phones = state.phones;
  const multi = phones.length > 1;
  const parts = [];

  if (multi) {
    const active = !state.view.phoneId;
    parts.push(`
      <button class="device-chip ${active ? 'on' : ''}" data-device="all" title="所有已配对手机">
        <svg viewBox="0 0 18 18" class="ic"><rect x="2.4" y="2.4" width="5.6" height="5.6" rx="1.8"/><rect x="10" y="2.4" width="5.6" height="5.6" rx="1.8"/><rect x="2.4" y="10" width="5.6" height="5.6" rx="1.8"/><rect x="10" y="10" width="5.6" height="5.6" rx="1.8"/></svg>
        <span class="d-name">全部设备</span>
        <span class="d-meta">${t('{n} 台', { n: phones.length })}</span>
      </button>`);
  }

  for (const p of phones) {
    const h = p.health;
    const active = !multi || state.view.phoneId === p.id;
    const meta = p.authError
      ? '需重新配对'
      : [
        h && typeof h.battery === 'number' ? `${h.battery}%` : '',
        p.lastSyncAt ? fmtAgo(p.lastSyncAt) : '未同步',
      ].filter(Boolean).join(' · ');
    parts.push(`
      <button class="device-chip ${active ? 'on' : ''}" data-device="${p.id}"
              title="${esc(p.name)} · ${esc(p.url)}${p.lastError ? ' · ' + esc(p.lastError) : ''}">
        <span class="status-dot ${deviceStateDot(p)}"></span>
        <span class="d-name" data-no-i18n>${esc(p.name)}</span>
        <span class="d-meta">${esc(meta)}</span>
        <span class="d-sync" data-sync-device="${p.id}" title="立即同步这台手机">${ICONS.sync}</span>
      </button>`);
  }

  if (!phones.length) {
    parts.push(`<span class="device-hint">还没有配对设备，点右侧「配对手机」开始</span>`);
  }
  wrap.innerHTML = parts.join('');
}

function renderSideDevice() {
  const el = $('#sideDevice');
  if (!el) return;
  const phones = state.phones;
  if (!phones.length) {
    el.innerHTML = `<div class="sd-empty">还没有配对设备<br /><span>点上方「配对手机」开始</span></div>`;
    return;
  }
  const scopeId = state.view.phoneId || (phones.length === 1 ? phones[0].id : null);
  const p = scopeId ? phones.find((x) => x.id === scopeId) : null;
  const unread = state.messages.filter((m) => m.type === 'inbox' && !m.read && !m.seen).length;

  if (!p) {
    el.innerHTML = `
      <div class="sd-card">
        <div class="sd-top"><span class="sd-icon">${ICONS.all}</span><span class="sd-name">全部设备</span></div>
        <div class="sd-sub">${t('{n} 台手机 · {m} 条短信', { n: phones.length, m: state.messages.length })}</div>
        <div class="sd-grid">
          <div><span>未读</span><b>${unread}</b></div>
          <div><span>在线</span><b>${phones.filter((x) => deviceStateDot(x) === 'online').length}</b></div>
        </div>
      </div>`;
    return;
  }

  const h = p.health;
  const sub = [
    p.url.replace(/^https?:\/\//, ''),
    h && typeof h.battery === 'number' ? t('电量 {n}%', { n: h.battery }) : '',
  ].filter(Boolean).join(' · ');
  const mine = state.messages.filter((m) => m.phoneId === p.id);
  el.innerHTML = `
    <div class="sd-card">
      <div class="sd-top"><span class="status-dot ${deviceStateDot(p)}"></span><span class="sd-name" data-no-i18n>${esc(p.name)}</span></div>
      <div class="sd-sub">${esc(sub)}</div>
      <div class="sd-grid">
        <div><span>短信</span><b>${mine.length}</b></div>
        <div><span>未读</span><b>${mine.filter((m) => m.type === 'inbox' && !m.read && !m.seen).length}</b></div>
      </div>
    </div>`;
}

function renderStatus() {
  const el = $('#sideStatus');
  if (!el) return;
  const phones = state.phones;
  if (!phones.length) { el.hidden = true; return; }
  el.hidden = false;
  const synced = phones.map((p) => p.lastSyncAt).filter(Boolean);
  const latest = synced.length ? Math.max(...synced) : 0;
  const interval = Number(state.settings.syncIntervalMin) || 0;
  const unread = state.messages.filter((m) => m.type === 'inbox' && !m.read && !m.seen).length;
  el.innerHTML = `
    <div class="ss-row"><span>本地短信</span><b>${state.messages.length.toLocaleString(localeTag())}</b></div>
    <div class="ss-row"><span>未读</span><b>${unread}</b></div>
    <div class="ss-row"><span>上次同步</span><b>${esc(fmtAgo(latest) || '—')}</b></div>
    <div class="ss-row"><span>自动同步</span><b>${interval ? t('每 {n} 分钟', { n: interval }) : t('已关闭')}</b></div>`;
}

function renderList() {
  const convs = buildConversations();
  const listEl = $('#convList');
  const emptyEl = $('#listEmpty');

  if (!state.phones.length) {
    listEl.innerHTML = '';
    emptyEl.hidden = false;
    emptyEl.innerHTML = `<b>${t('还没有配对任何手机')}</b>
      ${t('先在手机上打开短信桥 App 并启用服务，然后点击右上角「配对手机」。')}`;
    return;
  }
  if (!convs.length) {
    listEl.innerHTML = '';
    emptyEl.hidden = false;
    emptyEl.innerHTML = state.view.query
      ? `${t('没有匹配的短信')}<br />${t('换个关键词试试')}`
      : state.messages.length
        ? t('这个视图下还没有短信')
        : `${t('还没有同步任何短信')}<br />${t('点上方同步按钮开始')}`;
    return;
  }

  emptyEl.hidden = true;
  listEl.innerHTML = convs.map((c, i) => {
    const last = c.last;
    const phone = state.phones.find((p) => p.id === last.phoneId);
    const preview = (last.body || '').replace(/\s+/g, ' ').trim();
    const dir = last.type === 'sent' ? t('我：') : '';
    // the device badge is only meaningful while browsing all devices
    const showPhone = phone && !state.view.phoneId && state.phones.length > 1;
    const simKeys = new Set(c.messages.map(simKeyOf).filter(Boolean));
    const simChip = state.view.sim ? '' : simKeys.size > 1
      ? `<span class="chip sim">${t('{n} 张卡', { n: simKeys.size })}</span>`
      : simKeys.size === 1 ? `<span class="chip sim">${esc(simLabelOf(last))}</span>` : '';
    const chips = [
      showPhone ? `<span class="chip">${esc(phone.name)}</span>` : '',
      simChip,
      c.count > 1 ? `<span class="chip">${t('{n} 条', { n: c.count })}</span>` : '',
    ].filter(Boolean).join('');
    return `
      <div class="conv ${state.selected === c.key ? 'on' : ''} ${c.unread ? 'unread' : ''}"
           role="option" aria-selected="${state.selected === c.key}"
           tabindex="-1"
           data-conv="${esc(c.key)}" style="animation-delay:${Math.min(i, 12) * 18}ms">
        <span class="avatar" style="${avatarStyle(c.key)}">${esc(initials(c.key))}</span>
        <div class="conv-body">
          <div class="conv-top">
            <span class="conv-name">${esc(shortAddress(c.key))}</span>
            <span class="conv-time">${esc(fmtListTime(last.date))}</span>
            ${c.unread ? `<span class="unread-dot">${c.unread}</span>` : ''}
          </div>
          <div class="conv-preview" data-no-i18n>${esc(dir + (preview || '（空短信）'))}</div>
          ${chips ? `<div class="conv-bottom">${chips}</div>` : ''}
        </div>
      </div>`;
  }).join('');
}

function renderThread() {
  const threadEl = $('#thread');
  const emptyEl = $('#threadEmpty');
  const key = state.selected;
  const convs = buildConversations();
  const conv = convs.find((c) => c.key === key);

  if (!conv) {
    threadEl.hidden = true;
    emptyEl.hidden = false;
    return;
  }
  threadEl.hidden = false;
  emptyEl.hidden = true;

  const last = conv.last;
  const phone = state.phones.find((p) => p.id === last.phoneId);
  $('#threadAvatar').textContent = initials(conv.key);
  $('#threadAvatar').setAttribute('style', avatarStyle(conv.key));
  $('#threadTitle').textContent = shortAddress(conv.key);
  const bits = [];
  if (phone) bits.push(`<span class="chip">${esc(phone.name)}</span>`);
  if (last.carrier) bits.push(`<span class="chip">${esc(last.carrier)}</span>`);
  const seenSims = [];
  for (const m of conv.messages) {
    const k = simKeyOf(m);
    if (!k || seenSims.some((s) => s.key === k)) continue;
    seenSims.push({ key: k, label: simLabelOf(m), number: m.simNumber || '' });
  }
  if (seenSims.length === 1) {
    const num = fmtPhone(seenSims[0].number);
    bits.push(`<span class="chip sim">${esc(seenSims[0].label)}${num ? ` · ${esc(num)}` : ''}</span>`);
  } else if (seenSims.length > 1) {
    bits.push(`<span class="chip sim">${t('{n} 张卡', { n: seenSims.length })}</span>`);
  }
  bits.push(`<span class="metric">${esc(t('{n} 条 · 最后一条 {when}', { n: conv.count, when: fmtAgo(conv.last.date) }))}</span>`);
  $('#threadSub').innerHTML = bits.join('');

  let html = '';
  let lastDay = '';
  for (const m of conv.messages) {
    const dk = dayKey(m.date);
    if (dk !== lastDay) {
      html += `<div class="day">${esc(fmtDayLabel(m.date))}</div>`;
      lastDay = dk;
    }
    const bodyHtml = esc(String(m.body || '').replace(/\s+$/, '')).replace(/\n{3,}/g, '\n\n');
    const bubbleBody = bodyHtml || '<i style="opacity:.5">（空短信）</i>';
    const card = seenSims.length > 1 ? `${esc(simLabelOf(m))} · ` : '';
    html += `<div class="bubble ${m.type === 'sent' ? 'out' : 'in'}"><span class="body-text" data-no-i18n>${bubbleBody}</span><span class="stamp">${card}${esc(fmtClock(m.date))}${m.type === 'sent' ? t(' · 已发送') : ''}</span></div>`;
  }
  const scroller = $('#threadScroll');
  scroller.innerHTML = `<div class="bubbles">${html}</div>`;
  requestAnimationFrame(() => { scroller.scrollTop = scroller.scrollHeight; });
}

/** Move the roving selection without opening the thread (no markSeen). */
function selectConversation(key) {
  state.selected = key;
  renderList();
  renderThread();
}

/** Open a conversation from a pointer click or an Enter / Space / → press. */
function openConversation(key) {
  selectConversation(key);
  window.sms.markSeen(key).then((n) => {
    if (!n) return;
    for (const m of state.messages) if (m.address === key) m.seen = true;
    renderNav();
    renderList();
    renderStatus();
  }).catch(() => {});
}

function renderMeta() {
  const phones = state.phones;
  const withSync = phones.map((p) => p.lastSyncAt).filter(Boolean);
  const latest = withSync.length ? Math.max(...withSync) : 0;
  const interval = Number(state.settings.syncIntervalMin) || 0;
  const scoped = state.view.phoneId ? phones.find((p) => p.id === state.view.phoneId) : null;
  const scopeLabel = scoped ? scoped.name : phones.length > 1 ? t('全部设备') : phones[0] && phones[0].name;
  const parts = scopeLabel ? [scopeLabel] : [];
  parts.push(fmtRelative(latest));
  parts.push(interval ? t('每 {n} 分钟自动同步', { n: interval }) : t('自动同步已关闭'));
  $('#listMetaText').textContent = state.syncing ? t('正在同步…') : localizeText(parts.join(' · '));

  const btn = $('#btnSync');
  btn.classList.toggle('syncing', Boolean(state.syncing));
  const label = $('#syncLabel');
  if (label) label.textContent = state.syncing ? t('同步中') : t('同步');
  btn.title = scoped
    ? t('同步「{name}」（Ctrl+R）', { name: scoped.name })
    : phones.length > 1 ? t('同步全部设备（Ctrl+R）') : t('立即同步（Ctrl+R）');
}

/**
 * Keep the sidebar's bottom actions reachable at any display scale: if the
 * column would overflow, drop the secondary cards (status first, then the
 * device summary) instead of letting the footer slide out of view.
 */
function fitSidebar() {
  const sb = document.querySelector('.sidebar');
  if (!sb) return;
  sb.classList.remove('compact', 'compact-2');
  if (sb.scrollHeight > sb.clientHeight + 1) sb.classList.add('compact');
  if (sb.scrollHeight > sb.clientHeight + 1) sb.classList.add('compact-2');
}

function renderAll() {
  renderNav();
  renderDevices();
  renderSideDevice();
  renderStatus();
  renderList();
  renderThread();
  renderMeta();
  requestAnimationFrame(() => { localizeDom(document.body); fitSidebar(); });
  startLocalizer();
}

/** Watch for re-renders and translate them; started once the language is known. */
let localizerStarted = false;
function startLocalizer() {
  if (localizerStarted || typeof MutationObserver !== 'function') return;
  localizerStarted = true;
  // The observer must run even when the app boots in Chinese: a runtime switch
  // to en/ja would otherwise never localise anything rendered afterwards.
  let queued = false;
  const observer = new MutationObserver(() => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      if (window.I18N.getLang() !== 'zh') localizeDom(document.body);
    });
  });
  observer.observe(document.body, { childList: true, subtree: true });
}

/* ---------------------------------------------------------------- modal ─── */

let modalCloser = null;
let modalReturnFocus = null;
let modalBuild = null;

const FOCUSABLE = 'button:not([disabled]):not([tabindex="-1"]), [href], input:not([disabled]), '
  + 'select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function openModal(render, { onMount, wide = false } = {}) {
  const layer = $('#modalLayer');
  const card = $('#modalCard');
  const opener = document.activeElement;
  const html = typeof render === 'function' ? render() : render;
  modalBuild = () => openModal(render, { onMount, wide });
  card.className = `modal glass${wide ? ' wide' : ''}`;
  // The scrollable body and the action footer are separate boxes, so the last
  // form row can never be covered by the buttons.
  card.innerHTML = `<div class="modal-body">${html}</div>`;
  const actions = card.querySelector('.modal-body > .modal-actions');
  if (actions) card.appendChild(actions);
  localizeDom(card);
  layer.hidden = false;
  modalReturnFocus = opener instanceof HTMLElement ? opener : null;
  modalCloser = () => {
    layer.hidden = true;
    card.innerHTML = '';
    modalCloser = null;
    modalBuild = null;
    const back = modalReturnFocus;
    modalReturnFocus = null;
    if (back && document.contains(back)) back.focus();
  };
  card.querySelectorAll('[data-close]').forEach((b) => b.addEventListener('click', closeModal));
  if (onMount) onMount(card);
  trapFocus(card);
  const first = card.querySelector(FOCUSABLE);
  if (first) first.focus();
}

/**
 * Re-render the open dialog in the current language. Strings produced by t()
 * at build time cannot be retranslated by the DOM walker, so the dialog has to
 * be rebuilt from its builder.
 */
function rebuildModal() {
  const layer = $('#modalLayer');
  if (!layer || layer.hidden || !modalBuild) return;
  const card = $('#modalCard');
  const body = card.querySelector('.modal-body');
  const scroll = body ? body.scrollTop : 0;
  const active = document.activeElement;
  const activeId = active && active.id ? active.id : '';
  const activeCls = active && typeof active.className === 'string' ? active.className.trim() : '';
  const build = modalBuild;
  const ret = modalReturnFocus;
  layer.hidden = true;
  card.innerHTML = '';
  modalCloser = null;
  modalBuild = null;
  build();
  modalReturnFocus = ret;
  const body2 = card.querySelector('.modal-body');
  if (body2) body2.scrollTop = scroll;
  let next = null;
  if (activeId) next = card.querySelector(`#${activeId}`);
  if (!next && activeCls) next = card.querySelector(`.${activeCls.split(/\s+/)[0]}`);
  if (next && typeof next.focus === 'function') next.focus();
}

/** Keep Tab inside the dialog; the dialog itself closes on Esc (global keymap). */
function trapFocus(card) {
  if (card.dataset.trap === '1') return;
  card.dataset.trap = '1';
  card.addEventListener('keydown', (e) => {
    if (e.key !== 'Tab') return;
    const items = [...card.querySelectorAll(FOCUSABLE)].filter((el) => el.offsetParent !== null);
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  });
}

function closeModal() { if (modalCloser) modalCloser(); }

/** Lightweight glass context menu anchored to the pointer. */
let ctxMenuEl = null;
function closeContextMenu() {
  if (ctxMenuEl) { ctxMenuEl.remove(); ctxMenuEl = null; }
}
function showContextMenu(x, y, items) {
  closeContextMenu();
  const el = document.createElement('div');
  el.className = 'ctx-menu';
  for (const it of items) {
    if (it.sep) { const hr = document.createElement('div'); hr.className = 'ctx-sep'; el.appendChild(hr); continue; }
    const b = document.createElement('button');
    b.className = `ctx-item${it.danger ? ' danger' : ''}`;
    b.innerHTML = `${it.icon || ''}<span></span>`;
    b.querySelector('span').textContent = it.label;
    b.addEventListener('click', () => { closeContextMenu(); it.run(); });
    el.appendChild(b);
  }
  document.body.appendChild(el);
  const rect = el.getBoundingClientRect();
  el.style.left = `${Math.min(x, window.innerWidth - rect.width - 12)}px`;
  el.style.top = `${Math.min(y, window.innerHeight - rect.height - 12)}px`;
  ctxMenuEl = el;
}

/**
 * One persistent toast node: screen readers get a stable live region and only
 * the text changes. Re-showing it toggles [hidden], which restarts the CSS
 * entrance animation without rebuilding the element.
 */
let toastEl = null;
let toastTimer = 0;

function ensureToast() {
  const box = $('#toasts');
  box.setAttribute('role', 'status');
  box.setAttribute('aria-live', 'polite');
  box.setAttribute('aria-atomic', 'true');
  if (!toastEl) {
    toastEl = document.createElement('div');
    toastEl.className = 'toast';
    toastEl.hidden = true;
    box.appendChild(toastEl);
  }
  return toastEl;
}

function toast(message, kind = '') {
  const el = ensureToast();
  clearTimeout(toastTimer);
  el.className = `toast ${kind}`.trim();
  el.textContent = localizeText(message);
  el.classList.remove('out');
  el.hidden = false;
  toastTimer = setTimeout(() => {
    el.classList.add('out');
    toastTimer = setTimeout(() => {
      el.hidden = true;
      el.classList.remove('out');
    }, 340);
  }, kind === 'err' ? 4200 : 2400);
}

/** Hide and wipe the persistent toast (used when the language changes). */
function clearToast() {
  clearTimeout(toastTimer);
  if (!toastEl) return;
  toastEl.hidden = true;
  toastEl.classList.remove('out');
  toastEl.textContent = '';
}

/* ----------------------------------------------------------- phone modal ── */

function openAddPhone(opts = {}) {
  const repairing = opts.phoneId ? state.phones.find((p) => p.id === opts.phoneId) : null;
  const lastUrl = opts.url || repairing?.url || state.phones[0]?.url || 'https://192.168.1.5:8787';
  const lastCodeHint = repairing
    ? `这台手机的旧密钥已失效（换证书后会这样）。在手机上重新生成配对码，配对后会沿用现有的短信缓存。`
    : '配对成功后，这台电脑会获得一枚独立密钥，手机的配对列表里可以看到并随时吊销它。';
  openModal(() => `
    <div class="modal-title">
      <span class="logo" style="width:34px;height:34px;border-radius:12px;">
        <svg viewBox="0 0 18 18" width="17" height="17" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round">
          <rect x="5" y="2" width="8" height="14" rx="2.2"/><path d="M8 13.6h2"/>
        </svg>
      </span>
      <div>
        <h3>${repairing ? '重新配对' : '配对手机'}</h3>
        <p>${repairing ? esc(t('为「{name}」换一枚新密钥', { name: repairing.name })) : t('在手机上打开短信桥 → 启用服务 → 配对，获取 6 位配对码')}</p>
      </div>
      <button class="icon-btn modal-close" data-close title="关闭">
        <svg viewBox="0 0 16 16" class="ic"><path d="M4 4l8 8M12 4l-8 8"/></svg>
      </button>
    </div>

    <div class="tabs" id="pairTabs" ${repairing ? 'hidden' : ''}>
      <button class="on" data-tab="code">配对码</button>
      <button data-tab="import">已有凭据</button>
    </div>

    <div id="tabCode" style="display:flex;flex-direction:column;gap:14px;">
      <div class="field">
        <label>手机名称</label>
        <input class="input" id="pName" placeholder="例如：主力机 · 一加" value="${esc(repairing?.name || '')}" ${repairing ? 'readonly' : ''} />
      </div>
      <div class="field">
        <label>手机地址（IP:端口）</label>
        <input class="input" id="pUrl" placeholder="https://192.168.1.5:8787" value="${esc(lastUrl)}" spellcheck="false" />
      </div>
      <div class="field">
        <label>6 位配对码（5 分钟内有效）</label>
        <input class="input mono" id="pCode" inputmode="numeric" maxlength="6" placeholder="······" />
      </div>
      <div class="hint-line" id="pairHint">${esc(lastCodeHint)}</div>
    </div>

    <div id="tabImport" style="display:none;flex-direction:column;gap:14px;">
      <div class="field">
        <label>手机名称</label>
        <input class="input" id="iName" placeholder="例如：备用机" />
      </div>
      <div class="field">
        <label>手机地址</label>
        <input class="input" id="iUrl" placeholder="https://192.168.1.5:8787" value="${esc(lastUrl)}" spellcheck="false" />
      </div>
      <div class="field">
        <label>client_id</label>
        <input class="input" id="iClient" placeholder="uuid" spellcheck="false" />
      </div>
      <div class="field">
        <label>token</label>
        <input class="input" id="iToken" placeholder="base64url token" spellcheck="false" />
      </div>
      <div class="hint-line">${t('可从已有的 .smsbridge.json 复制 base_url / client_id / token，凭据会用 Windows DPAPI 加密后保存在本机。')}</div>
    </div>

    <div class="modal-actions">
      <button class="btn ghost" id="btnProbe">测试连接</button>
      <button class="btn primary" id="btnPairGo">配对并同步</button>
    </div>
  `, {
    onMount(card) {
      const tabs = card.querySelector('#pairTabs');
      const codePane = card.querySelector('#tabCode');
      const importPane = card.querySelector('#tabImport');
      const goBtn = card.querySelector('#btnPairGo');
      tabs.addEventListener('click', (e) => {
        const b = e.target.closest('[data-tab]');
        if (!b) return;
        tabs.querySelectorAll('button').forEach((x) => x.classList.toggle('on', x === b));
        const isCode = b.dataset.tab === 'code';
        codePane.style.display = isCode ? 'flex' : 'none';
        importPane.style.display = isCode ? 'none' : 'flex';
        goBtn.textContent = isCode ? t('配对并同步') : t('导入并同步');
      });

      const code = card.querySelector('#pCode');
      code.addEventListener('input', () => {
        code.value = code.value.replace(/\D/g, '').slice(0, 6);
      });
      code.addEventListener('keydown', (e) => { if (e.key === 'Enter') goBtn.click(); });
      setTimeout(() => card.querySelector('#pUrl').focus(), 60);

      const hint = card.querySelector('#pairHint');
      const setHint = (text, cls = '') => { hint.className = `hint-line ${cls}`; hint.innerHTML = text; };

      card.querySelector('#btnProbe').addEventListener('click', async (e) => {
        const btn = e.currentTarget;
        btn.disabled = true;
        setHint('正在连接手机…');
        try {
          const codeTab = card.querySelector('#tabCode').style.display !== 'none';
          const url = (codeTab ? card.querySelector('#pUrl') : card.querySelector('#iUrl')).value;
          const h = await window.sms.probe(url);
          setHint(
            t('连接成功 · {model} · Android {android} · 电量 {battery}% · 已配对 {clients}', {
              model: `<b>${esc(h.model || t('未知机型'))}</b>`,
              android: esc(h.android),
              battery: h.battery,
              clients: h.paired_clients,
            })
            + `<br /><span class="fp">${esc(t('证书指纹 {fp}', { fp: h.fingerprint || '—' }))}</span>`,
            'ok',
          );
          const nameInput = card.querySelector('#pName');
          if (!nameInput.value && h.model) nameInput.value = h.model;
        } catch (err) {
          setHint(esc(errText(err) || '连接失败'), 'err');
        } finally {
          btn.disabled = false;
        }
      });

      goBtn.addEventListener('click', async () => {
        const isCode = tabs.querySelector('.on').dataset.tab === 'code';
        goBtn.disabled = true;
        const original = goBtn.textContent;
        goBtn.textContent = '处理中…';
        try {
          if (isCode) {
            const url = card.querySelector('#pUrl').value.trim();
            const name = card.querySelector('#pName').value.trim();
            const c = card.querySelector('#pCode').value.trim();
            if (c.length !== 6) throw new Error('请输入手机上显示的 6 位配对码');
            setHint('正在配对…');
            const phone = await window.sms.pair({ name, url, code: c, phoneId: opts.phoneId || null });
            closeModal();
            const fp = phone.fingerprint ? t('，证书 {fp}…', { fp: phone.fingerprint.slice(0, 11) }) : '';
            toast(t('已配对「{name}」{fp}，正在拉取短信', { name: phone.name, fp }), 'ok');
            await reload();
          } else {
            const payload = {
              name: card.querySelector('#iName').value.trim(),
              url: card.querySelector('#iUrl').value.trim(),
              clientId: card.querySelector('#iClient').value.trim(),
              token: card.querySelector('#iToken').value.trim(),
            };
            if (!payload.clientId || !payload.token) throw new Error('请填写 client_id 和 token');
            setHint('正在验证凭据…');
            const phone = await window.sms.importPhone(payload);
            closeModal();
            toast(`已导入「${phone.name}」`, 'ok');
            await reload();
          }
        } catch (err) {
          setHint(esc(errText(err)), 'err');
          goBtn.disabled = false;
          goBtn.textContent = original;
        }
      });
    },
  });
}

/* -------------------------------------------------------- settings modal ── */

function openSettings() {
  const s = state.settings;
  const opt = (v, cur, label) => `<option value="${v}" ${String(cur) === String(v) ? 'selected' : ''}>${label}</option>`;
  openModal(() => `
    <div class="modal-title">
      <div>
        <h3>设置</h3>
        <p>同步节奏、开机启动与本地数据</p>
      </div>
      <button class="icon-btn modal-close" data-close title="关闭">
        <svg viewBox="0 0 16 16" class="ic"><path d="M4 4l8 8M12 4l-8 8"/></svg>
      </button>
    </div>

    <div class="setting-row">
      <div class="setting-copy"><b>外观主题</b><span>深色玻璃 / 冷白玻璃 / 米白纸感</span></div>
      <div class="segmented" id="setTheme">
        <button data-theme-set="dark" class="${(s.theme || 'dark') === 'dark' ? 'on' : ''}">深色</button>
        <button data-theme-set="light" class="${s.theme === 'light' ? 'on' : ''}">冷白</button>
        <button data-theme-set="paper" class="${s.theme === 'paper' ? 'on' : ''}">米白</button>
      </div>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>语言</b><span>界面语言，切换后立即生效</span></div>
      <select class="select" id="setLang">
        ${['auto', 'zh', 'en', 'ja'].map((id) => `<option value="${id}" ${(s.lang || 'auto') === id ? 'selected' : ''}>${
          id === 'auto' ? '自动' : id === 'zh' ? '简体中文' : id === 'en' ? 'English' : '日本語'}</option>`).join('')}
      </select>
    </div>
    <div class="setting-row">
      <div class="setting-copy">
        <b>界面缩放</b>
        <span>整体放大或缩小界面（文字、间距一起变），觉得字小就调大</span>
      </div>
      <div class="stepper" id="setScale">
        <button data-scale="-5" title="${t('缩小 5%')}">A−</button>
        <span id="scaleValue">${Math.round(scaleOf(s) * 100)}%</span>
        <button data-scale="5" title="${t('放大 5%')}">A+</button>
      </div>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>开机自动启动</b><span>登录 Windows 后静默在托盘运行</span></div>
      <button class="switch ${s.autoStart ? 'on' : ''}" role="switch" aria-checked="${Boolean(s.autoStart)}" aria-label="开机自动启动" data-toggle="autoStart"></button>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>启动后最小化</b><span>不弹出主窗口，只在托盘里待命</span></div>
      <button class="switch ${s.startHidden ? 'on' : ''}" role="switch" aria-checked="${Boolean(s.startHidden)}" aria-label="启动后最小化" data-toggle="startHidden"></button>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>关闭窗口时最小化到托盘</b><span>保持后台同步，不退出程序</span></div>
      <button class="switch ${s.closeToTray ? 'on' : ''}" role="switch" aria-checked="${Boolean(s.closeToTray)}" aria-label="关闭窗口时最小化到托盘" data-toggle="closeToTray"></button>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>启动时自动同步</b><span>打开程序后立刻拉取一次</span></div>
      <button class="switch ${s.syncOnStart ? 'on' : ''}" role="switch" aria-checked="${Boolean(s.syncOnStart)}" aria-label="启动时自动同步" data-toggle="syncOnStart"></button>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>新短信桌面通知</b><span>程序在后台时弹出系统通知</span></div>
      <button class="switch ${s.notifyNew ? 'on' : ''}" role="switch" aria-checked="${Boolean(s.notifyNew)}" aria-label="新短信桌面通知" data-toggle="notifyNew"></button>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>定时同步</b><span>后台按间隔自动拉取新短信</span></div>
      <select class="select" id="setInterval">
        ${opt(0, s.syncIntervalMin, '关闭')}
        ${opt(1, s.syncIntervalMin, '每分钟')}
        ${opt(5, s.syncIntervalMin, '每 5 分钟')}
        ${opt(15, s.syncIntervalMin, '每 15 分钟')}
        ${opt(30, s.syncIntervalMin, '每 30 分钟')}
        ${opt(60, s.syncIntervalMin, '每小时')}
      </select>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>每次同步回溯</b><span>从手机读取最近多少天的短信（手机端上限 30 天）</span></div>
      <select class="select" id="setPullDays">
        ${opt(1, s.pullDays, '1 天')}
        ${opt(3, s.pullDays, '3 天')}
        ${opt(7, s.pullDays, '7 天')}
        ${opt(30, s.pullDays, '30 天')}
      </select>
    </div>
    <div class="setting-row">
      <div class="setting-copy"><b>本地保留</b><span>超过期限的本地副本自动清理</span></div>
      <select class="select" id="setKeepDays">
        ${opt(0, s.keepDays, '永久')}
        ${opt(30, s.keepDays, '30 天')}
        ${opt(90, s.keepDays, '90 天')}
        ${opt(365, s.keepDays, '1 年')}
      </select>
    </div>

    <div class="hint-line">
      ${t('本地消息 {n} 条 · 数据目录 {dir}', { n: `<b>${state.stats.total}</b>`, dir: `<b>${esc(state.app.dataDir || '')}</b>` })}<br />
      配对密钥使用 Windows DPAPI 加密存储，仅当前系统账户可解。
    </div>

    <div class="modal-actions" style="justify-content:space-between;">
      <div style="display:flex;gap:8px;">
        <button class="btn ghost" id="btnOpenData">数据目录</button>
        <button class="btn ghost" id="btnExportAll">导出 CSV</button>
      </div>
      <button class="btn danger" id="btnClear">清空本地缓存</button>
    </div>
  `, {
    onMount(card) {
      const scaleBox = card.querySelector('#setScale');
      if (scaleBox) {
        const label = card.querySelector('#scaleValue');
        const paint = () => { label.textContent = `${Math.round(scaleOf(state.settings) * 100)}%`; };
        scaleBox.addEventListener('click', async (e) => {
          const b = e.target.closest('[data-scale]');
          if (!b) return;
          const next = clampScale(Number(state.settings.uiScale || 1) + Number(b.dataset.scale) / 100);
          state.settings.uiScale = next;
          await window.sms.setSettings({ uiScale: next });
          paint();
        });
        scaleBox.addEventListener('dblclick', async () => {
          state.settings.uiScale = 1;
          await window.sms.setSettings({ uiScale: 1 });
          paint();
        });
      }

      card.querySelectorAll('[data-toggle]').forEach((btn) => {
        btn.addEventListener('click', async () => {
          const key = btn.dataset.toggle;
          const next = !btn.classList.contains('on');
          btn.classList.toggle('on', next);
          btn.setAttribute('aria-checked', String(next));
          state.settings = await window.sms.setSettings({ [key]: next });
          if (key === 'autoStart') {
            toast(next ? '已开启开机自启' : '已关闭开机自启', 'ok');
          }
        });
      });
      card.querySelectorAll('#setTheme [data-theme-set]').forEach((b) => {
        b.addEventListener('click', () => {
          const next = b.dataset.themeSet;
          applyTheme(next, { persist: true });
          card.querySelectorAll('#setTheme [data-theme-set]').forEach((x) => x.classList.toggle('on', x === b));
        });
      });
      card.querySelector('#setLang').addEventListener('change', async (e) => {
        state.settings = await window.sms.setSettings({ lang: e.target.value });
        applyLanguage();
      });

      card.querySelector('#setInterval').addEventListener('change', async (e) => {
        state.settings = await window.sms.setSettings({ syncIntervalMin: Number(e.target.value) });
        renderMeta();
        toast(Number(e.target.value) ? t('定时同步：每 {n} 分钟', { n: e.target.value }) : t('已关闭定时同步'), 'ok');
      });
      card.querySelector('#setPullDays').addEventListener('change', async (e) => {
        state.settings = await window.sms.setSettings({ pullDays: Number(e.target.value) });
      });
      card.querySelector('#setKeepDays').addEventListener('change', async (e) => {
        state.settings = await window.sms.setSettings({ keepDays: Number(e.target.value) });
      });
      card.querySelector('#btnOpenData').addEventListener('click', () => window.sms.openDataDir());
      card.querySelector('#btnExportAll').addEventListener('click', async () => {
        const r = await window.sms.exportMessages(null);
        if (r.saved) toast(t('已导出 {n} 条到 CSV', { n: r.count }), 'ok');
      });
      card.querySelector('#btnClear').addEventListener('click', async () => {
        const snap = await window.sms.clearMessages(null);
        applyState(snap);
        toast('本地缓存已清空', 'ok');
      });
    },
    wide: true,
  });
}

function openAbout() {
  const selfRepo = REPO_SELF
    ? `<a class="link-out" data-external="${esc(REPO_SELF)}">Windows SMS Bridge<svg viewBox="0 0 16 16" class="lo-ic"><path d="M6.2 3.4h6.4v6.4M12.6 3.4 4.2 11.8"/></svg></a>
       <span class="lo-note">本程序 · MIT</span>`
    : `<span class="lo-pending">本程序仓库：GitHub 推送后在此显示</span>`;

  openModal(() => `
    <div class="modal-title">
      <span class="logo" style="width:36px;height:36px;border-radius:12px;">${brandMark()}</span>
      <div>
        <h3>Windows SMS Bridge <span style="font-size:11px;color:var(--dim);">v${esc(state.app.version || '1.0.0')}</span></h3>
        <p>短信桥 · Windows 端客户端</p>
      </div>
      <button class="icon-btn modal-close" data-close>
        <svg viewBox="0 0 16 16" class="ic"><path d="M4 4l8 8M12 4l-8 8"/></svg>
      </button>
    </div>
    <div class="hint-line">
      <b>传输方式</b><br />
      局域网内 HTTPS（手机端自签名证书）+ HMAC-SHA256 请求签名。首次配对会把手机证书固定下来
      （TOFU），之后每次请求都用这张证书校验，正文全程加密；配对弹窗里的指纹可与手机配对页逐段比对。
      签名另行保证「请求确实来自已配对客户端、内容未被篡改」。跨网络请走 Tailscale / WireGuard。
    </div>
    <div class="hint-line">
      <b>本地存储</b><br />
      短信缓存在本机 userData 目录；配对 token 由 Windows DPAPI 加密，仅当前 Windows 账户可解密；
      手机证书指纹随设备记录保存。本程序只读，不会向手机写入任何内容。
    </div>
    <div class="hint-line">
      <b>开源与致谢</b><br />
      <a class="link-out" data-external="${esc(REPO_PHONE)}">android-agent-sms-bridge<svg viewBox="0 0 16 16" class="lo-ic"><path d="M6.2 3.4h6.4v6.4M12.6 3.4 4.2 11.8"/></svg></a>
      <span class="lo-note">手机端短信桥 · MIT</span><br />
      ${selfRepo}
    </div>
    <div class="modal-actions">
      <button class="btn primary" data-close>关闭</button>
    </div>
  `);
}

/* ------------------------------------------------------------------ sync ── */

async function runSync({ phoneId = null } = {}) {
  if (state.syncing) return;
  if (!state.phones.length) { openAddPhone(); return; }
  state.syncing = true;
  renderMeta();
  try {
    let summary;
    if (phoneId) {
      const res = await window.sms.syncOne(phoneId);
      summary = res.state.sync.lastSummary;
      applyState(res.state);
    } else {
      const res = await window.sms.syncAll();
      summary = res.summary;
      applyState(res.state);
    }
    const added = summary?.added || 0;
    const errs = summary?.errors?.length || 0;
    if (errs && !added) {
      toast(summary.errors[0].message || '同步失败', 'err');
    } else if (errs) {
      toast(t('部分同步成功 · 新增 {a} 条 · {e} 台设备失败', { a: added, e: errs }), 'err');
    } else {
      toast(added ? t('同步完成 · 新增 {n} 条短信', { n: added }) : t('同步完成 · 没有新短信'), 'ok');
    }
  } catch (err) {
    toast(errText(err), 'err');
  } finally {
    state.syncing = false;
    renderAll();
  }
}

/* ----------------------------------------------------------------- state ── */

function applyState(snap) {
  if (!snap) return;
  state.phones = snap.phones || [];
  state.messages = snap.messages || [];
  state.settings = snap.settings || {};
  state.stats = snap.stats || { total: 0 };
  state.sync = snap.sync || {};
  state.app = snap.app || {};
  state.syncing = Boolean(snap.sync?.running);

  const lang = resolveLang(state.settings.lang, navigator.language);
  const changed = setLang(lang);
  document.documentElement.lang = lang === 'zh' ? 'zh-CN' : lang;
  if (changed) document.title = t('Windows SMS Bridge · 短信桥');
}

/** Locale tag for toLocaleString(), so 1,234 / 1.234 follow the UI language. */
function localeTag() {
  return { zh: 'zh-CN', ja: 'ja-JP', en: 'en-US' }[window.I18N.getLang()] || 'en-US';
}

/** Switch language on the live DOM — no window reload, so state survives. */
function applyLanguage() {
  const lang = resolveLang(state.settings.lang, navigator.language);
  setLang(lang);
  document.documentElement.lang = lang === 'zh' ? 'zh-CN' : lang;
  document.title = t('Windows SMS Bridge · 短信桥');
  rebuildModal();
  relocalizeDom(document.body);
  renderAll();
  // a toast is a single persistent node, so its last text would otherwise stay
  // in the previous language
  clearToast();
}

async function reload() {
  const snap = await window.sms.getState();
  applyState(snap);
  renderAll();
}

/* ------------------------------------------------------------------ boot ── */

function bindEvents() {
  // window controls
  document.querySelector('.titlebar').addEventListener('dblclick', (e) => {
    if (e.target.closest('button')) return;
    window.sms.win.toggleMaximize();
  });
  $('#btnMin').addEventListener('click', () => window.sms.win.minimize());
  $('#btnMax').addEventListener('click', () => window.sms.win.toggleMaximize());
  $('#btnClose').addEventListener('click', () => window.sms.win.close());

  // nav
  $('#navFilters').addEventListener('click', (e) => {
    const b = e.target.closest('[data-mode]');
    if (!b) return;
    state.view.mode = b.dataset.mode;
    renderAll();
  });

  // SIM filter (only rendered when the selected phone has 2+ cards)
  $('#navSims').addEventListener('click', (e) => {
    const b = e.target.closest('[data-sim]');
    if (!b) return;
    state.view.sim = b.dataset.sim || null;
    state.selected = null;
    renderAll();
  });

  // search
  const search = $('#searchInput');
  let t = null;
  search.addEventListener('input', () => {
    clearTimeout(t);
    t = setTimeout(() => { state.view.query = search.value; renderList(); }, 140);
  });

  // device rail (top level: pick a phone first, then a view)
  $('#deviceChips').addEventListener('click', async (e) => {
    const syncBtn = e.target.closest('[data-sync-device]');
    if (syncBtn) {
      e.stopPropagation();
      await runSync({ phoneId: syncBtn.dataset.syncDevice });
      return;
    }
    const chip = e.target.closest('[data-device]');
    if (!chip) return;
    const id = chip.dataset.device;
    if (id === 'all') {
      state.view.phoneId = null;
    } else {
      state.view.phoneId = state.view.phoneId === id ? null : id;
    }
    state.selected = null;
    state.view.sim = null;
    renderAll();
    if (state.view.phoneId) window.sms.phoneHealth(state.view.phoneId).then(() => reload()).catch(() => {});
  });

  $('#btnAddDevice').addEventListener('click', openAddPhone);

  $('#btnTheme').addEventListener('click', () => {
    const next = THEME_NEXT[document.documentElement.dataset.theme] || 'light';
    applyTheme(next, { persist: true });
    toast(t('已切换到' + THEME_LABEL[next] + '主题'), 'ok');
  });

  document.addEventListener('click', (e) => {
    const link = e.target.closest('[data-external]');
    if (!link) return;
    e.preventDefault();
    window.sms.openExternal(link.dataset.external);
  });
  $('#btnSettings').addEventListener('click', openSettings);
  $('#btnAbout').addEventListener('click', openAbout);

  // list interactions
  $('#convList').addEventListener('click', (e) => {
    const conv = e.target.closest('[data-conv]');
    if (!conv) return;
    openConversation(conv.dataset.conv);
  });

  // listbox keyboard model: one tab stop for the whole list, arrows move the
  // roving tabindex (and the selection, so the thread pane follows along)
  const convList = $('#convList');
  convList.setAttribute('role', 'listbox');
  convList.tabIndex = 0;
  convList.addEventListener('keydown', (e) => {
    const rows = [...convList.querySelectorAll('[data-conv]')];
    if (!rows.length) return;
    const cur = rows.findIndex((r) => r.dataset.conv === state.selected);
    const focusRow = (key) => {
      // renderList replaced the nodes; put focus back on the same row
      const again = [...convList.querySelectorAll('[data-conv]')]
        .find((r) => r.dataset.conv === key);
      if (again) again.focus();
    };
    const move = (i) => {
      const row = rows[Math.max(0, Math.min(rows.length - 1, i))];
      if (!row) return;
      // arrows only move the roving focus/selection; opening a thread (and
      // marking it seen) stays on Enter / Space / →
      selectConversation(row.dataset.conv);
      focusRow(row.dataset.conv);
    };
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); move(cur + 1); break;
      case 'ArrowUp': e.preventDefault(); move(cur < 0 ? 0 : cur - 1); break;
      case 'Home': e.preventDefault(); move(0); break;
      case 'End': e.preventDefault(); move(rows.length - 1); break;
      case 'Enter':
      case ' ':
      case 'ArrowRight': {
        const row = rows[cur < 0 ? 0 : cur];
        if (!row) return;
        e.preventDefault();
        openConversation(row.dataset.conv);
        focusRow(row.dataset.conv);
        break;
      }
      default:
    }
  });

  // pause the ambient blob animation while the window sits in the tray
  const setIdle = () => document.body.classList.toggle('is-idle', document.hidden);
  document.addEventListener('visibilitychange', setIdle);
  setIdle();

  // device context menu
  $('#deviceChips').addEventListener('contextmenu', (e) => {
    const card = e.target.closest('[data-device]');
    if (!card || card.dataset.device === 'all') return;
    e.preventDefault();
    const id = card.dataset.device;
    const phone = state.phones.find((p) => p.id === id);
    if (!phone) return;
    showContextMenu(e.clientX, e.clientY, [
      { label: '立即同步', icon: ICONS.sync, run: () => runSync({ phoneId: id }) },
      { label: '复制地址', run: async () => { await navigator.clipboard.writeText(phone.url); toast('已复制地址', 'ok'); } },
      { label: '重新配对（换新密钥/证书）', icon: ICONS.key, run: () => openAddPhone({ phoneId: id }) },
      { sep: true },
      { label: '重命名', run: async () => {
        const name = prompt(t('给这台手机起个名字'), phone.name);
        if (!name) return;
        await window.sms.renamePhone(id, name.trim());
        await reload();
      } },
      { label: '删除这台手机', danger: true, icon: ICONS.trash, run: async () => {
        await window.sms.removePhone(id);
        if (state.view.phoneId === id) state.view.phoneId = null;
        if (state.selected) state.selected = null;
        await reload();
        toast('已移除', 'ok');
      } },
    ]);
  });

  // sync buttons
  $('#btnSync').addEventListener('click', () => runSync());

  $('#btnCopyNumber').addEventListener('click', async () => {
    if (!state.selected) return;
    await navigator.clipboard.writeText(shortAddress(state.selected));
    toast('号码已复制', 'ok');
  });

  $('#btnExportConv').addEventListener('click', async () => {
    if (!state.selected) return;
    const phoneId = state.view.phoneId || null;
    const r = await window.sms.exportMessages(phoneId);
    if (r.saved) toast(t('已导出 {n} 条', { n: r.count }), 'ok');
  });

  document.addEventListener('click', (e) => {
    if (ctxMenuEl && !e.target.closest('.ctx-menu')) closeContextMenu();
  });
  window.addEventListener('blur', closeContextMenu);
  window.addEventListener('resize', () => requestAnimationFrame(fitSidebar));

  // Pointer highlight: one rAF-throttled listener writes --mx/--my on the panel
  // under the cursor; pointerleave drops them so the sheen fades back to its
  // default spot. Alpha lives in CSS (--sheen-alpha, 0.06).
  let sheenRaf = 0;
  let sheenPanel = null;
  let sheenX = 0;
  let sheenY = 0;
  const panelOf = (target) => (target && target.closest ? target.closest('.panel') : null);
  const resetSheen = (panel) => {
    if (!panel) return;
    panel.style.removeProperty('--mx');
    panel.style.removeProperty('--my');
  };
  document.addEventListener('pointermove', (e) => {
    const panel = panelOf(e.target);
    if (!panel) return;
    if (sheenPanel && sheenPanel !== panel) resetSheen(sheenPanel);
    sheenPanel = panel;
    sheenX = e.clientX;
    sheenY = e.clientY;
    if (sheenRaf) return;
    sheenRaf = requestAnimationFrame(() => {
      sheenRaf = 0;
      if (!sheenPanel) return;
      const r = sheenPanel.getBoundingClientRect();
      sheenPanel.style.setProperty('--mx', `${Math.round(sheenX - r.left)}px`);
      sheenPanel.style.setProperty('--my', `${Math.round(sheenY - r.top)}px`);
    });
  });
  document.addEventListener('pointerleave', (e) => {
    const panel = panelOf(e.target);
    if (panel) resetSheen(panel);
    if (panel && panel === sheenPanel) sheenPanel = null;
  }, true);

  // Any render (list, thread, toast, modal) inserts Chinese copy; translate it
  // right after the mutation instead of remembering to call localizeDom at
  // every call site. childList only — our own text edits would cause a loop.


  // modal layer close
  $('#modalLayer').addEventListener('click', (e) => {
    if (e.target.dataset.close !== undefined) closeModal();
  });

  // keyboard
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      if (ctxMenuEl) { closeContextMenu(); return; }
      if (!$('#modalLayer').hidden) closeModal();
      else if (state.selected) { state.selected = null; renderList(); renderThread(); }
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      search.focus();
      search.select();
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'r') {
      e.preventDefault();
      runSync({ full: false });
    }
  });
}

function bindIpc() {
  window.sms.onStateChanged(async () => { await reload(); });
  window.sms.onSyncStarted(() => { state.syncing = true; renderMeta(); });
  window.sms.onSyncProgress((p) => {
    if (p.phase === 'fetching') {
      $('#listMetaText').textContent = t('正在从 {name} 读取 · 已获取 {n} 条', {
        name: state.phones.find((x) => x.id === p.phoneId)?.name || t('手机'),
        n: p.count,
      });
    }
  });
  window.sms.onSyncError((e) => {
    if (e.message) toast(`${e.name || '手机'}：${e.message}`, 'err');
  });
  window.sms.onSettingsUpdate((s) => { state.settings = s; });
  window.sms.onMaximized(() => {});
}

async function init() {
  bindEvents();
  bindIpc();
  ensureToast();

  // brand logo
  $('#brandLogo').innerHTML = brandMark();

  await reload();
  state.loading = false;

  applyTheme(state.settings.theme || 'dark');
  const ver = $('#brandVersion');
  if (ver && state.app.version) ver.textContent = `v${state.app.version}`;

  // onboarding when there is nothing paired yet
  if (!state.phones.length) {
    setTimeout(() => {
      openModal(() => `
        <div class="onboard">
          <div class="orb-illustration"><span></span><span></span><span></span></div>
          <div>
            <h3 style="font-size:18px;font-weight:650;letter-spacing:.2px;">${t('欢迎使用 Windows SMS Bridge')}</h3>
            <p style="margin-top:8px;font-size:12.6px;color:var(--dim);line-height:1.9;">
              ${t('把这台电脑和手机连起来，短信会自动同步到本地。')}<br />${t('支持同时配对多台手机。')}
            </p>
          </div>
          <div class="steps">
            <div class="step"><i>1</i><div>${t('手机安装 {app}，点「启用服务」并授予短信权限', { app: `<b>${t('短信桥 (SmsBridge)')}</b>` })}</div></div>
            <div class="step"><i>2</i><div>${t('确保电脑与手机在 {wifi}，记下手机上显示的 IP', { wifi: `<b>${t('同一 Wi-Fi')}</b>` })}</div></div>
            <div class="step"><i>3</i><div>${t('打开手机的 {pair} 页面，把 6 位配对码填进来', { pair: `<b>${t('配对')}</b>` })}</div></div>
          </div>
          <div class="modal-actions" style="width:100%;justify-content:center;">
            <button class="btn ghost" data-close>${t('稍后再说')}</button>
            <button class="btn primary" id="obGo" style="flex:1;max-width:220px;">${t('开始配对')}</button>
          </div>
        </div>
      `, {
        onMount(card) {
          card.querySelector('#obGo').addEventListener('click', () => { closeModal(); setTimeout(openAddPhone, 120); });
        },
      });
    }, 420);
  }
}

/** Fatal boot failure fallback. Also driven by scripts/i18n-sweep.js. */
function renderCrash(err) {
  const msg = err && err.message ? err.message : String(err);
  document.body.innerHTML = '<div style="padding:40px;font-family:sans-serif;color:var(--text);">'
    + `${esc(t('启动失败：{msg}', { msg }))}</div>`;
}

init().catch((err) => {
  console.error(err);
  renderCrash(err);
});

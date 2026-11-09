'use strict';
/**
 * Sync engine — pulls SMS from paired phones into the local cache.
 *
 * Two modes:
 *   full        re-read the last N days (used on first sync / manual deep sync)
 *   incremental only messages with _id > lastSmsId (cheap, the default)
 *
 * The phone enforces 60 req/min per client and caps days/limit server-side, so
 * we page with limit=200 and stay well under that.
 */

const { BridgeClient } = require('./smsbridge');
const { t } = require('../shared/i18n');

const PAGE_SIZE = 200;
const MAX_PAGES = 20;

const inflight = new Map();

function normalize(row) {
  return {
    id: row.id,
    address: row.address || '',
    body: row.body || '',
    date: row.date || 0,
    type: row.type || 'inbox',
    slot: typeof row.slot === 'number' ? row.slot : -1,
    subId: row.sub_id ?? 0,
    carrier: row.carrier || '',
    simNumber: row.sim_number || '',
    read: Boolean(row.read),
  };
}

async function fetchPages(client, baseParams, onPage) {
  const rows = [];
  for (let page = 0; page < MAX_PAGES; page++) {
    const res = await client.sms({ ...baseParams, limit: PAGE_SIZE, offset: page * PAGE_SIZE });
    const list = Array.isArray(res.messages) ? res.messages : [];
    rows.push(...list);
    if (onPage) onPage(rows.length, res);
    if (list.length < PAGE_SIZE) break;
  }
  return rows;
}

async function syncPhone(store, phoneId, { full = false, days = null, onProgress = null } = {}) {
  const phone = store.getPhone(phoneId);
  if (!phone) throw new Error(t('手机不存在'));
  const token = store.tokenOf(phoneId);
  if (!token) throw new Error(t('这台手机还没有配对凭据，请重新配对'));

  if (inflight.has(phoneId)) return inflight.get(phoneId);

  const task = (async () => {
    const client = new BridgeClient({
      baseUrl: phone.url,
      clientId: phone.clientId,
      token,
      certPem: phone.certPem || null,
    });
    const params = { box: 'all' };
    const sinceId = Number(phone.lastSmsId) || 0;
    const mode = full || !sinceId ? 'full' : 'incremental';

    if (mode === 'full') {
      params.days = Number(days ?? store.settings.pullDays) || 30;
    } else {
      params.sinceId = sinceId;
    }

    onProgress?.({ phoneId, phase: 'fetching', mode, count: 0 });
    const raw = await fetchPages(client, params, (n) => onProgress?.({ phoneId, phase: 'fetching', mode, count: n }));
    const rows = raw.map(normalize);

    const result = store.upsertMessages(phoneId, rows);
    const maxId = rows.reduce((m, r) => Math.max(m, r.id), sinceId);

    store.updatePhone(phoneId, {
      lastSmsId: maxId,
      lastSyncAt: Date.now(),
      lastError: '',
      lastMode: mode,
    });

    onProgress?.({ phoneId, phase: 'done', ...result });
    return { phoneId, mode, ...result, fetched: rows.length, lastSmsId: maxId };
  })().finally(() => inflight.delete(phoneId));

  inflight.set(phoneId, task);
  return task;
}

module.exports = { syncPhone, normalize };

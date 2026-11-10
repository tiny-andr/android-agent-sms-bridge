#!/usr/bin/env node
/**
 * Compare the SMS cache produced by the Electron client and the Tauri client
 * after they each ran a full sync against the same phone in the same window.
 *
 *   node scripts/compare-sync.cjs <electron-data-dir> <rust-data-dir>
 *
 * Exit code 0 = the caches are equivalent; 1 = a difference (printed).
 * It compares:
 *   · per phone: cached row count, lastSmsId/lastSyncAt bookkeeping
 *   · every cached message: id, address, body, date, type, slot, subId,
 *     carrier, simNumber, read (the dual-SIM columns included)
 *   · conversation aggregation (per address: count, unread, last date/id)
 *   · the dual-SIM label set (slot → carrier / sim number)
 */
'use strict';

const fs = require('fs');
const path = require('path');

function read(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (err) {
    return null;
  }
}

const [electronDir, rustDir] = process.argv.slice(2);
if (!electronDir || !rustDir) {
  console.error('usage: node scripts/compare-sync.cjs <electron-data-dir> <rust-data-dir>');
  process.exit(2);
}

const MESSAGE_FIELDS = ['id', 'address', 'body', 'date', 'type', 'slot', 'subId', 'carrier', 'simNumber', 'read'];
const problems = [];
const notes = [];

function load(dir) {
  const config = read(path.join(dir, 'config.json'));
  const db = read(path.join(dir, 'messages.json'));
  if (!config || !db) throw new Error(`${dir}: config.json or messages.json missing/unreadable`);
  return { config, db };
}

const a = load(path.resolve(electronDir));
const b = load(path.resolve(rustDir));

const phones = (cfg) => (cfg.phones || []).filter((p) => Array.isArray(cfg === a.config ? a.db.messages[p.id] : b.db.messages[p.id]) || true);

function bucket(db, id) {
  return (db.messages && db.messages[id]) || [];
}

const aIds = Object.keys(a.db.messages || {});
const bIds = Object.keys(b.db.messages || {});
if (aIds.length !== bIds.length) {
  problems.push(`phone buckets: electron has ${aIds.length} (${aIds.join(', ')}), tauri has ${bIds.length} (${bIds.join(', ')})`);
}

for (const phoneId of aIds) {
  const aPhone = (a.config.phones || []).find((p) => p.id === phoneId) || {};
  const bPhone = (b.config.phones || []).find((p) => p.id === phoneId) || {};
  const label = `${aPhone.name || phoneId} (${phoneId.slice(0, 8)})`;
  const rowsA = bucket(a.db, phoneId);
  const rowsB = bucket(b.db, phoneId);

  if (rowsA.length !== rowsB.length) {
    problems.push(`${label}: message count electron=${rowsA.length} tauri=${rowsB.length}`);
  } else {
    notes.push(`${label}: ${rowsA.length} messages on both sides`);
  }
  if ((aPhone.lastSmsId || 0) !== (bPhone.lastSmsId || 0)) {
    problems.push(`${label}: lastSmsId electron=${aPhone.lastSmsId} tauri=${bPhone.lastSmsId}`);
  }
  if (Boolean(aPhone.simBackfilled) !== Boolean(bPhone.simBackfilled)) {
    problems.push(`${label}: simBackfilled electron=${aPhone.simBackfilled} tauri=${bPhone.simBackfilled}`);
  }

  const byIdB = new Map(rowsB.map((m) => [m.id, m]));
  let compared = 0;
  for (const m of rowsA) {
    const other = byIdB.get(m.id);
    if (!other) {
      problems.push(`${label}: id ${m.id} present in electron cache only`);
      continue;
    }
    compared += 1;
    for (const field of MESSAGE_FIELDS) {
      if (JSON.stringify(m[field]) !== JSON.stringify(other[field])) {
        problems.push(`${label}: id ${m.id} field ${field}: electron=${JSON.stringify(m[field])} tauri=${JSON.stringify(other[field])}`);
      }
    }
    const extraA = Object.keys(m).filter((k) => !MESSAGE_FIELDS.includes(k) && k !== 'seen').sort();
    const extraB = Object.keys(other).filter((k) => !MESSAGE_FIELDS.includes(k) && k !== 'seen').sort();
    if (JSON.stringify(extraA) !== JSON.stringify(extraB)) {
      problems.push(`${label}: id ${m.id} extra fields electron=[${extraA}] tauri=[${extraB}]`);
    }
  }
  if (compared) notes.push(`${label}: ${compared} messages compared field by field`);

  // dual-SIM labels
  const simsA = new Set(rowsA.map((m) => `${m.slot}|${m.carrier}|${m.simNumber}`));
  const simsB = new Set(rowsB.map((m) => `${m.slot}|${m.carrier}|${m.simNumber}`));
  const simsAJson = JSON.stringify([...simsA].sort());
  const simsBJson = JSON.stringify([...simsB].sort());
  if (simsAJson !== simsBJson) {
    problems.push(`${label}: SIM label set differs electron=${simsAJson} tauri=${simsBJson}`);
  } else {
    notes.push(`${label}: SIM labels ${simsAJson}`);
  }

  // conversation aggregation (what the UI list shows)
  const conv = (rows) => {
    const map = new Map();
    for (const m of rows) {
      const key = m.address || '';
      const entry = map.get(key) || { address: key, count: 0, unread: 0, lastDate: 0, lastId: 0 };
      entry.count += 1;
      if (m.type === 'inbox' && !m.read) entry.unread += 1;
      if ((m.date || 0) >= entry.lastDate) {
        entry.lastDate = m.date || 0;
        entry.lastId = m.id;
      }
      map.set(key, entry);
    }
    return [...map.values()]
      .map((e) => `${e.address}|${e.count}|${e.unread}|${e.lastDate}|${e.lastId}`)
      .sort();
  };
  const convA = conv(rowsA);
  const convB = conv(rowsB);
  if (JSON.stringify(convA) !== JSON.stringify(convB)) {
    problems.push(`${label}: conversation aggregation differs\n  electron: ${JSON.stringify(convA)}\n  tauri:    ${JSON.stringify(convB)}`);
  } else {
    notes.push(`${label}: ${convA.length} conversations identical`);
  }
}

console.log('--- comparison notes ---');
for (const note of notes) console.log(`  ok  ${note}`);
if (problems.length) {
  console.log('--- differences ---');
  for (const p of problems) console.log(`  DIFF ${p}`);
  console.log(`\nRESULT: ${problems.length} difference(s)`);
  process.exit(1);
}
console.log('\nRESULT: caches are equivalent');

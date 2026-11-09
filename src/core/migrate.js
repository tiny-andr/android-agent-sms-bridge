'use strict';
/**
 * Profile recovery across the rename "SMS Glass" -> "Windows SMS Bridge".
 *
 * It runs on every start, not just the first one: an early build could have
 * created an empty profile before the copy happened, and losing a pairing means
 * losing the only copy of the DPAPI-sealed token.
 *
 * Rules
 *  · never overwrite a target profile that already has phones; merge instead
 *  · de-duplicate by phone id and by message id, so running twice is harmless
 *  · keep the target's settings (theme, autostart) intact
 */

const fs = require('node:fs');
const path = require('node:path');

function readJson(file) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return null; }
}

function recoverProfile({ appData, appName, legacyNames = ['SMS Glass'], log = () => {} }) {
  const result = { phones: 0, messages: 0, from: '' };
  const target = path.join(appData, appName);

  for (const legacyName of legacyNames) {
    const legacy = path.join(appData, legacyName);
    const srcCfg = readJson(path.join(legacy, 'config.json'));
    if (!srcCfg || !Array.isArray(srcCfg.phones) || !srcCfg.phones.length) continue;

    const dstCfgPath = path.join(target, 'config.json');
    const dstCfg = readJson(dstCfgPath);
    fs.mkdirSync(target, { recursive: true });

    if (!dstCfg) {
      fs.copyFileSync(path.join(legacy, 'config.json'), dstCfgPath);
      result.phones += srcCfg.phones.length;
      log('[migrate] profile copied from', legacy);
    } else {
      const have = new Set((dstCfg.phones || []).map((p) => p.id));
      const missing = srcCfg.phones.filter((p) => !have.has(p.id));
      if (missing.length) {
        dstCfg.phones = [...(dstCfg.phones || []), ...missing];
        fs.writeFileSync(dstCfgPath, JSON.stringify(dstCfg, null, 2), 'utf8');
        result.phones += missing.length;
        log('[migrate] recovered %d pairing(s) from %s', missing.length, legacy);
      }
    }

    // cached messages: union per phone, de-duplicated by the phone's own _id
    const srcMsgs = readJson(path.join(legacy, 'messages.json'));
    if (srcMsgs && srcMsgs.messages) {
      const dstMsgsPath = path.join(target, 'messages.json');
      const dstMsgs = readJson(dstMsgsPath) || { version: 1, messages: {} };
      dstMsgs.messages = dstMsgs.messages || {};
      let added = 0;
      for (const [phoneId, list] of Object.entries(srcMsgs.messages)) {
        const current = dstMsgs.messages[phoneId] || [];
        const ids = new Set(current.map((m) => m.id));
        const extra = list.filter((m) => !ids.has(m.id));
        if (!extra.length) continue;
        dstMsgs.messages[phoneId] = [...current, ...extra].sort((a, b) => a.id - b.id);
        added += extra.length;
      }
      if (added) {
        fs.writeFileSync(dstMsgsPath, JSON.stringify(dstMsgs), 'utf8');
        result.messages += added;
        log('[migrate] recovered %d cached message(s)', added);
      }
    }

    if (!result.from) result.from = legacyName;
  }

  return result;
}

module.exports = { recoverProfile };

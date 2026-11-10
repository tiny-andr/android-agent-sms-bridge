#!/usr/bin/env node
/**
 * Prepares the two throwaway profiles for scripts/parity-sync.ps1.
 *
 * Both clients get the real pairings (tokenEnc + certPem + Local State, so DPAPI
 * keeps working) but a clean SMS cache and reset sync cursors, which makes each
 * do a FULL 30-day pull — that is the interesting comparison, not the no-op
 * incremental sync.
 *
 * JSON is written with Node (no BOM, platform-independent formatting) because
 * the Windows PowerShell `Set-Content -Encoding utf8` writes a BOM that
 * `JSON.parse` rejects, which silently empties both profiles.
 *
 *   node scripts/prepare-parity.cjs <appdata-profile> <electron-dir> <rust-dir>
 */
'use strict';

const fs = require('fs');
const path = require('path');

const [appDataDir, electronDir, rustDir] = process.argv.slice(2);
if (!appDataDir || !electronDir || !rustDir) {
  console.error('usage: node scripts/prepare-parity.cjs <appdata-profile> <electron-dir> <rust-dir>');
  process.exit(2);
}

const source = path.join(appDataDir, 'config.json');
const localState = path.join(appDataDir, 'Local State');
const config = JSON.parse(fs.readFileSync(source, 'utf8'));

// keep only phones whose token can actually be opened with this profile's
// Local State; the rest would only produce identical "re-pair" errors on both
// sides and make the comparison noisy
const phones = (config.phones || []).filter((p) => typeof p.tokenEnc === 'string' && p.tokenEnc);
if (!phones.length) throw new Error('no paired phone with an encrypted token in ' + source);

for (const phone of phones) {
  phone.lastSmsId = 0;
  phone.lastSyncAt = 0;
  phone.simBackfilled = false;
  phone.lastError = '';
  phone.authError = false;
}
const prepared = {
  ...config,
  phones,
  settings: {
    ...config.settings,
    syncOnStart: true,
    syncIntervalMin: 0,
  },
};
delete prepared.legacyMigrated;
delete prepared.removedPhoneIds;

for (const dir of [electronDir, rustDir]) {
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'config.json'), JSON.stringify(prepared, null, 2), 'utf8');
  fs.copyFileSync(localState, path.join(dir, 'Local State'));
  fs.writeFileSync(path.join(dir, 'messages.json'), JSON.stringify({ version: 1, messages: {} }), 'utf8');
}

console.log(`[parity] prepared 2 profiles with ${phones.length} phone(s): ${phones.map((p) => p.name).join(', ')}`);

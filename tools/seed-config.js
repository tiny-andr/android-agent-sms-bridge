/**
 * Dev-only helper: seed this app's config with credentials that already exist in
 * the phone-bridge repo, so the UI can be tested without a new pairing code.
 * Usage: node tools/seed-config.js <smsbridge.json> [display name]
 */
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const src = process.argv[2];
const name = process.argv[3] || '主力机';
if (!src) {
  console.error('usage: node tools/seed-config.js <smsbridge.json> [name]');
  process.exit(1);
}
const creds = JSON.parse(fs.readFileSync(src, 'utf8'));
const dir = path.join(process.env.APPDATA || process.env.HOME, 'Windows SMS Bridge');
fs.mkdirSync(dir, { recursive: true });

const configPath = path.join(dir, 'config.json');
let config = { version: 1, settings: {}, phones: [] };
if (fs.existsSync(configPath)) {
  try { config = JSON.parse(fs.readFileSync(configPath, 'utf8')); } catch { /* reset */ }
}

const exists = config.phones.some((p) => p.url === creds.base_url && p.clientId === creds.client_id);
if (!exists) {
  config.phones.push({
    id: crypto.randomUUID(),
    name,
    url: creds.base_url,
    clientId: creds.client_id,
    clientName: creds.client_name || '',
    createdAt: Date.now(),
    lastSyncAt: 0,
    lastSeq: 0,
    lastSmsId: 0,
    tokenRaw: creds.token,
    tokenEnc: '',
  });
}
fs.writeFileSync(configPath, JSON.stringify(config, null, 2), 'utf8');
console.log('seeded', configPath, 'phones:', config.phones.length);

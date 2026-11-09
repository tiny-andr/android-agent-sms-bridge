'use strict';
/**
 * Update pipeline for the Windows client.
 *
 * The GitHub release is both the version manifest and the trust anchor: its
 * `digest` field carries the SHA-256 of every asset, so a mirror can only ever
 * serve a byte-identical installer. Mirrors are tried in order for the
 * download only; nothing about the version check is delegated to them.
 */

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn } = require('node:child_process');

const API = 'https://api.github.com/repos/tiny-andr/android-agent-sms-bridge/releases/latest';
const ASSET_RE = /^Windows-SMS-Bridge-Setup-.*\.exe$/;

/** Empty prefix = GitHub direct, the rest are public mainland mirrors. */
const SOURCES = [
  '',
  'https://ghfast.top/',
  'https://gh-proxy.com/',
  'https://ghproxy.net/',
  'https://ghproxy.site/',
];

const UA = 'Windows-SMS-Bridge-Updater';

function compareVersions(a, b) {
  const pa = String(a || '').split('.').map((x) => parseInt(x, 10) || 0);
  const pb = String(b || '').split('.').map((x) => parseInt(x, 10) || 0);
  const n = Math.max(pa.length, pb.length);
  for (let i = 0; i < n; i += 1) {
    const x = pa[i] || 0;
    const y = pb[i] || 0;
    if (x !== y) return x > y ? 1 : -1;
  }
  return 0;
}

async function fetchJson(url, timeout = 8000) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeout);
  try {
    const res = await fetch(url, {
      signal: ctrl.signal,
      headers: { 'User-Agent': UA, Accept: 'application/vnd.github+json' },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return await res.json();
  } finally {
    clearTimeout(timer);
  }
}

/** @returns {Promise<null|{version:string, notes:string, asset:object}>} */
async function check(currentVersion) {
  const rel = await fetchJson(API);
  const version = String(rel.tag_name || '').replace(/^v/, '');
  if (!version || compareVersions(version, currentVersion) <= 0) return null;

  const assets = Array.isArray(rel.assets) ? rel.assets : [];
  const found = assets.find((a) => ASSET_RE.test(String(a.name || '')));
  if (!found) return null;

  const digest = String(found.digest || '');
  return {
    version,
    notes: String(rel.body || ''),
    asset: {
      name: found.name,
      size: Number(found.size) || 0,
      sha256: digest.startsWith('sha256:') ? digest.slice(7) : '',
      url: found.browser_download_url,
    },
  };
}

function sha256File(file) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(file);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('error', reject);
    stream.on('end', () => resolve(hash.digest('hex')));
  });
}

/**
 * Downloads the installer, falling through the mirror list on any failure.
 * @returns {Promise<string>} path of the verified file
 */
async function download(asset, dir, onProgress) {
  fs.mkdirSync(dir, { recursive: true });
  const target = path.join(dir, asset.name);
  let lastError = null;

  for (const prefix of SOURCES) {
    const label = prefix ? prefix.replace(/^https:\/\//, '').replace(/\/$/, '') : 'github';
    try {
      if (fs.existsSync(target)) fs.unlinkSync(target);
      const ctrl = new AbortController();
      const timer = setTimeout(() => ctrl.abort(), 15000);
      const res = await fetch(prefix + asset.url, {
        signal: ctrl.signal,
        headers: { 'User-Agent': UA },
      });
      clearTimeout(timer);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);

      const total = Number(res.headers.get('content-length')) || asset.size || 0;
      const out = fs.createWriteStream(target);
      let done = 0;
      for await (const chunk of res.body) {
        done += chunk.length;
        if (!out.write(chunk)) {
          await new Promise((r) => out.once('drain', r));
        }
        if (onProgress) onProgress(done, total);
      }
      await new Promise((resolve, reject) => {
        out.end((err) => (err ? reject(err) : resolve()));
      });

      if (asset.size > 0 && fs.statSync(target).size !== asset.size) {
        throw new Error(`size ${fs.statSync(target).size}/${asset.size}`);
      }
      if (asset.sha256) {
        const got = await sha256File(target);
        if (got.toLowerCase() !== asset.sha256.toLowerCase()) throw new Error('sha256 mismatch');
      }
      return target;
    } catch (err) {
      lastError = err;
      // eslint-disable-next-line no-console
      console.warn(`[updater] ${label} failed: ${err.message}`);
    }
  }
  throw new Error(lastError ? lastError.message : 'download failed');
}

/**
 * Runs the NSIS installer silently, then reopens the app. The installer cannot
 * replace files while we hold them, so it waits for this process to exit first.
 */
function installAndRestart(installer, exePath) {
  const cmd = [
    'timeout /t 2 /nobreak >nul',
    `"${installer}" /S`,
    `start "" "${exePath}"`,
  ].join(' & ');
  spawn('cmd.exe', ['/c', cmd], {
    detached: true,
    stdio: 'ignore',
    windowsHide: true,
  }).unref();
}

module.exports = { check, download, installAndRestart, compareVersions };

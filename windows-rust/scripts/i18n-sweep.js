#!/usr/bin/env node
/**
 * i18n sweep: loads the real renderer in an offscreen Electron window, switches
 * language at runtime (no reload) and fails when any copy did not follow.
 *
 *   npm run i18n:check
 *
 * Under plain node `require('electron')` resolves to the binary path, so the
 * script re-launches itself inside Electron. That keeps the npm script a simple
 * `node scripts/i18n-sweep.js`.
 */
'use strict';

const path = require('path');
const fs = require('fs');
const { pathToFileURL } = require('url');

const electronModule = require('electron');

if (typeof electronModule === 'string') {
  const { spawnSync } = require('child_process');
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  const res = spawnSync(electronModule, [__filename], { stdio: 'inherit', env });
  process.exit(res.status == null ? 1 : res.status);
}

const { app, BrowserWindow } = require('electron');

const ROOT = path.resolve(__dirname, '..');
// SWEEP_INDEX/SWEEP_OUT let the same sweep run against the Electron renderer
// (windows/src/ui) and the Tauri copy (web/ui) so their defect lists can be
// diffed - the rewrite must not add or remove i18n defects.
const INDEX_OVERRIDE = process.env.SWEEP_INDEX;
const OUT_OVERRIDE = process.env.SWEEP_OUT;
const INDEX = INDEX_OVERRIDE ? path.resolve(INDEX_OVERRIDE) : path.join(ROOT, 'web', 'ui', 'index.html');
const OUT_DIR = path.join(ROOT, '.shots', 'p0');
const OUT_FILE = OUT_OVERRIDE ? path.resolve(OUT_OVERRIDE) : path.join(OUT_DIR, 'i18n-report.json');
const PAGE_SCRIPT = fs.readFileSync(path.join(__dirname, 'sweep-page.js'), 'utf8');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function runDataset(dataset) {
  const win = new BrowserWindow({
    show: false,
    width: 1380,
    height: 880,
    webPreferences: {
      preload: path.join(__dirname, 'sweep-preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      backgroundThrottling: false,
      additionalArguments: ['--sweep-ds=' + dataset],
    },
  });
  win.webContents.on('did-fail-load', (e, code, desc, url, isMain) => {
    console.error('[i18n-sweep] did-fail-load', code, desc, url, 'main=' + isMain);
  });
  try {
    // dataset is passed through additionalArguments (a query on a file:// URL
    // is rejected by Chromium on Windows)
    await win.loadURL(pathToFileURL(INDEX).href);
    await sleep(dataset === 'empty' ? 1800 : 1000);
    return await win.webContents.executeJavaScript(PAGE_SCRIPT, true);
  } finally {
    if (!win.isDestroyed()) win.destroy();
  }
}

// destroying the first window must not end the sweep
app.on('window-all-closed', () => {});

app.whenReady().then(async () => {
  const records = [];
  try {
    for (const dataset of ['empty', 'paired']) {
      const part = await runDataset(dataset);
      records.push(...part);
    }
  } catch (err) {
    console.error('[i18n-sweep] failed to render:', err && err.message ? err.message : err);
    app.exit(2);
    return;
  }

  const scored = records.filter((r) => !r.meta);
  const byKind = {};
  let total = 0;
  let failedStates = 0;
  for (const rec of scored) {
    total += rec.defectCount;
    if (rec.defectCount > 0) failedStates += 1;
    for (const d of rec.defects) byKind[d.kind] = (byKind[d.kind] || 0) + 1;
  }

  const report = {
    generatedAt: new Date().toISOString(),
    scannedStates: scored.length,
    failedStates,
    totalDefects: total,
    defectsByKind: byKind,
    results: scored,
  };

  fs.mkdirSync(OUT_DIR, { recursive: true });
  fs.writeFileSync(OUT_FILE, `${JSON.stringify(report, null, 2)}\n`, 'utf8');

  console.log(`[i18n-sweep] states scanned: ${scored.length}, failed: ${failedStates}, defects: ${total}`);
  const kinds = Object.keys(byKind).sort();
  console.log(`[i18n-sweep] by kind: ${kinds.length ? kinds.map((k) => `${k}=${byKind[k]}`).join(' ') : 'none'}`);
  for (const rec of scored) {
    if (rec.defectCount === 0) continue;
    console.log(`  FAIL ${rec.state} [${rec.lang}] ${rec.defectCount}`);
    for (const d of rec.defects.slice(0, 8)) console.log(`    - ${d.kind} ${d.path} :: ${d.text}`);
    if (rec.defectCount > 8) console.log(`    ... ${rec.defectCount - 8} more`);
  }
  console.log(`[i18n-sweep] report: ${path.relative(ROOT, OUT_FILE)}`);

  app.exit(total === 0 ? 0 : 1);
});

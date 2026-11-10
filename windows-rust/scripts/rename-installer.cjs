#!/usr/bin/env node
/**
 * Tauri's NSIS bundler hardcodes the installer file name as
 * `<productName>_<version>_<arch>-setup.exe`. Everything else in the project
 * (README, BUILD.md, the in-app updater's `^Windows-SMS-Bridge-Setup-.*\.exe$`
 * asset rule, the release checklist) expects the Electron-style
 * `Windows-SMS-Bridge-Setup-<version>.exe`, so rename it right after bundling.
 *
 *   node scripts/rename-installer.cjs
 *
 * Exit code 0 = installer exists under the expected name (renamed or already
 * correct), 1 = no installer found.
 */
'use strict';

const fs = require('fs');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const pkg = JSON.parse(fs.readFileSync(path.join(ROOT, 'package.json'), 'utf8'));
const version = pkg.version;
const bundleDir = path.join(ROOT, 'target', 'release', 'bundle', 'nsis');
const expected = path.join(bundleDir, `Windows-SMS-Bridge-Setup-${version}.exe`);

if (!fs.existsSync(bundleDir)) {
  console.error(`[dist] bundle directory missing: ${bundleDir} (run "cargo tauri build" first)`);
  process.exit(1);
}

const candidates = fs
  .readdirSync(bundleDir)
  .filter((name) => /-setup\.exe$/i.test(name))
  .map((name) => path.join(bundleDir, name));
if (!candidates.length) {
  if (fs.existsSync(expected)) {
    console.log(`[dist] installer already named correctly: ${expected}`);
    process.exit(0);
  }
  console.error(`[dist] no NSIS installer in ${bundleDir}`);
  process.exit(1);
}

const source = candidates[0];
if (fs.existsSync(expected)) {
  // Always let the freshly bundled installer win over a previous rename.
  fs.rmSync(expected, { force: true });
}
fs.renameSync(source, expected);
console.log(`[dist] ${path.basename(source)} -> ${path.basename(expected)}`);
console.log(`[dist] installer: ${expected}`);

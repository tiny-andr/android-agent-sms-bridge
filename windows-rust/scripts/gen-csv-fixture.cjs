#!/usr/bin/env node
// Generates `scripts/csv-fixture.json`: the ground truth for the CSV export's
// timestamp column.
//
// The Electron build formatted a message date with
//   new Date(m.date).toLocaleString(locale)   // locale: zh-CN / ja-JP / en-US
// so the Rust port must reproduce Node's ICU output byte for byte (24h vs 12h,
// zero-padded vs bare hours, the comma in en-US, ...). This script asks the
// real Node ICU for those strings; `src/export.rs` unit-tests `format_timestamp`
// against the result. Re-run it whenever the fixture needs regenerating:
//
//   node scripts/gen-csv-fixture.cjs
//
// Wall-clock inputs are constructed with `new Date(y, mo-1, d, h, mi, s, ms)`,
// i.e. in the machine's local timezone — exactly what `chrono::Local` decodes.
// The fixture records each case's UTC offset so the Rust test can refuse to
// compare when it runs in a different timezone (regenerate there instead).

'use strict';

const fs = require('fs');
const path = require('path');

// app language id -> ICU locale used by Electron's i18n.getLang() ternary
const LOCALES = { zh: 'zh-CN', ja: 'ja-JP', en: 'en-US' };

// `label` is only for test failure messages; comments name the edge case.
const CASES = [
  // --- leap day 2024-02-29: midnight / noon / five past / last second
  { label: 'leap-day-midnight', y: 2024, mo: 2, d: 29, h: 0, mi: 0, s: 0, ms: 0 },
  { label: 'leap-day-noon', y: 2024, mo: 2, d: 29, h: 12, mi: 0, s: 0, ms: 0 },
  { label: 'leap-day-00-05-00', y: 2024, mo: 2, d: 29, h: 0, mi: 5, s: 0, ms: 0 },
  { label: 'leap-day-12-05-00', y: 2024, mo: 2, d: 29, h: 12, mi: 5, s: 0, ms: 0 },
  { label: 'leap-day-23-59-59', y: 2024, mo: 2, d: 29, h: 23, mi: 59, s: 59, ms: 0 },
  // sub-second precision is dropped by toLocaleString
  { label: 'leap-day-midnight-plus-789ms', y: 2024, mo: 2, d: 29, h: 0, mi: 0, s: 0, ms: 789 },

  // --- single-digit month / day / hour
  { label: 'single-digit-am', y: 2024, mo: 1, d: 2, h: 9, mi: 3, s: 4, ms: 0 },
  { label: 'single-digit-pm', y: 2024, mo: 1, d: 2, h: 21, mi: 3, s: 4, ms: 0 },
  { label: 'single-digit-early-morning', y: 2024, mo: 3, d: 5, h: 5, mi: 6, s: 7, ms: 0 },

  // --- year 2030
  { label: 'year-2030', y: 2030, mo: 1, d: 2, h: 3, mi: 4, s: 5, ms: 0 },
  { label: 'year-2030-end', y: 2030, mo: 12, d: 31, h: 23, mi: 59, s: 59, ms: 0 },

  // --- the app's own cache timeframe (2026-11-04)
  { label: 'app-cache-2246', y: 2026, mo: 11, d: 4, h: 22, mi: 46, s: 40, ms: 0 },
  { label: 'app-cache-1046', y: 2026, mo: 11, d: 4, h: 10, mi: 46, s: 40, ms: 0 },
  { label: 'app-cache-midnight', y: 2026, mo: 11, d: 4, h: 0, mi: 0, s: 0, ms: 0 },
  { label: 'app-cache-00-05-00', y: 2026, mo: 11, d: 4, h: 0, mi: 5, s: 0, ms: 0 },
  { label: 'app-cache-12-05-00', y: 2026, mo: 11, d: 4, h: 12, mi: 5, s: 0, ms: 0 },
  { label: 'app-cache-1pm', y: 2026, mo: 11, d: 4, h: 13, mi: 5, s: 9, ms: 0 },

  // --- epoch boundary / pre-epoch (negative ms)
  { label: 'epoch', y: 1970, mo: 1, d: 1, h: 8, mi: 0, s: 0, ms: 0 },
  { label: 'pre-epoch', y: 1969, mo: 12, d: 31, h: 23, mi: 59, s: 59, ms: 0 },

  // --- y2k
  { label: 'y2k-midnight', y: 2000, mo: 1, d: 1, h: 0, mi: 0, s: 0, ms: 0 },
];

function pad(n, width = 2) {
  return String(n).padStart(width, '0');
}

function localIso(c) {
  return (
    `${c.y}-${pad(c.mo)}-${pad(c.d)}T${pad(c.h)}:${pad(c.mi)}:${pad(c.s)}.` +
    `${pad(c.ms, 3)}`
  );
}

const cases = CASES.map((c) => {
  const dt = new Date(c.y, c.mo - 1, c.d, c.h, c.mi, c.s, c.ms);
  const ms = dt.getTime();
  if (!Number.isFinite(ms)) throw new Error(`invalid local time for ${c.label}`);
  // second pass guards against a DST fold silently shifting the wall clock
  if (
    dt.getFullYear() !== c.y ||
    dt.getMonth() !== c.mo - 1 ||
    dt.getDate() !== c.d ||
    dt.getHours() !== c.h ||
    dt.getMinutes() !== c.mi ||
    dt.getSeconds() !== c.s
  ) {
    throw new Error(
      `local time ${localIso(c)} is ambiguous in this timezone (DST gap/fold); ` +
        'pick a different wall clock for ' + c.label
    );
  }

  const shared = new Date(ms);
  if (shared.getTime() !== ms) throw new Error(`round-trip failed for ${c.label}`);

  const expected = {};
  for (const [lang, locale] of Object.entries(LOCALES)) {
    expected[lang] = new Date(ms).toLocaleString(locale);
  }

  return {
    label: c.label,
    ms,
    local: localIso(c),
    utcIso: dt.toISOString(),
    // UTC offset in minutes, positive east of UTC (+480 for Asia/Shanghai).
    // Rust compares this with chrono's local_minus_utc() / 60.
    utcOffsetMinutes: -dt.getTimezoneOffset(),
    expected,
  };
});

const fixture = {
  // Regenerate with `node scripts/gen-csv-fixture.cjs`; do not hand-edit.
  note:
    'Ground truth for src/export.rs format_timestamp(): real Node ICU output of ' +
    'Date#toLocaleString for the locales the Electron app used. Values are ' +
    'wall-clock-identical only in the recorded timezone.',
  locales: LOCALES,
  nodeVersion: process.version,
  icuVersion: process.versions.icu,
  timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  platform: process.platform,
  generatedAt: new Date().toISOString(),
  // Measured patterns (samples come from the cases below; keep in sync).
  patterns: {
    zh: 'yyyy/M/d HH:mm:ss  (24h, hour zero-padded)      -> 2026/11/4 00:05:00',
    ja: 'y/M/d H:mm:ss     (24h, hour NOT padded)       -> 2026/11/4 0:05:00',
    en: 'M/d/yyyy, h:mm:ss A (12h, hour NOT padded)     -> 11/4/2026, 12:05:00 AM',
  },
  cases,
};

const outPath = path.join(__dirname, 'csv-fixture.json');
fs.writeFileSync(outPath, JSON.stringify(fixture, null, 2) + '\n', 'utf8');

console.log(
  `wrote ${path.relative(process.cwd(), outPath)}: ${cases.length} cases x ` +
    `${Object.keys(LOCALES).length} locales`
);
console.log(
  `node ${fixture.nodeVersion}  ICU ${fixture.icuVersion}  tz ${fixture.timeZone}`
);
console.log('label'.padEnd(34) + 'zh'.padEnd(24) + 'ja'.padEnd(23) + 'en');
for (const c of cases) {
  console.log(
    c.label.padEnd(34) +
      c.expected.zh.padEnd(24) +
      c.expected.ja.padEnd(23) +
      c.expected.en
  );
}

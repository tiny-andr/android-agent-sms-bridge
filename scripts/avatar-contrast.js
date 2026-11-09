#!/usr/bin/env node
/**
 * Avatar contrast check.
 *
 * P2 replaced the per-address hue gradient with a monochrome glass tile: an 8%
 * tint fill over the panel, with the standard text colour as the ink. There is
 * no hue sweep any more, so this script checks the three theme pairings instead.
 *
 *   npm run avatar:contrast
 */
'use strict';

const TINT_ALPHA = 0.08;

// panel background and tokens, copied from styles.css
const THEMES = {
  dark: { panel: [0x20, 0x20, 0x24], tint: [255, 255, 255], text: [0xf3, 0xf4, 0xfa] },
  light: { panel: [0xfb, 0xfc, 0xfe], tint: [32, 38, 50], text: [0x17, 0x1a, 0x20] },
  paper: { panel: [0xfc, 0xfb, 0xf7], tint: [61, 50, 38], text: [0x1f, 0x1a, 0x14] },
};

function lin(c) {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

function lum(rgb) {
  return 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
}

function contrast(a, b) {
  const la = lum(a);
  const lb = lum(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

function over(fg, alpha, bg) {
  return fg.map((c, i) => c * alpha + bg[i] * (1 - alpha));
}

function hex(rgb) {
  return `#${rgb.map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')}`;
}

let failed = 0;
console.log('theme   fill               text      contrast');
for (const [name, t] of Object.entries(THEMES)) {
  const fill = over(t.tint, TINT_ALPHA, t.panel);
  const c = contrast(t.text, fill);
  const ok = c >= 4.5;
  if (!ok) failed += 1;
  console.log(`${name.padEnd(7)} ${hex(fill).padEnd(18)} ${hex(t.text)}  ${c.toFixed(2)}:1 ${ok ? 'PASS' : 'FAIL'}`);
}
console.log(failed === 0 ? '\nPASS: monochrome avatar tiles clear 4.5:1 in every theme' : '\nFAIL: raise --tint alpha or change the ink');
process.exit(failed === 0 ? 0 : 1);

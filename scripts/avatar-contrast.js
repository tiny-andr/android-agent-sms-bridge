#!/usr/bin/env node
/**
 * Avatar contrast sweep.
 *
 * avatarStyle() paints a two-stop light gradient and always uses dark ink, so
 * the guarantee has to hold across all 360 hues and across the whole tile, not
 * just at the glyph midpoint. This script reproduces the same hsl values and
 * reports the worst glyph-box contrast plus the per-hue table.
 *
 *   npm run avatar:contrast
 */
'use strict';

const INK = [0, 0, 0]; // --avatar-ink-dark
const LIGHTNESS = 0.66; // both stops share this band
const SAT_A = 0.72;
const SAT_B = 0.68;
const HUE_SHIFT = 42;

function hslToRgb(h, s, l) {
  const f = (n) => {
    const k = (n + h / 30) % 12;
    return l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  };
  return [f(0), f(8), f(4)];
}

function lin(c) {
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function lum(c) {
  return 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
}

function contrast(a, b) {
  const la = lum(a);
  const lb = lum(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** Worst contrast anywhere along the rendered gradient for one hue. */
function tileWorst(h) {
  const h2 = (h + HUE_SHIFT) % 360;
  const a = hslToRgb(h, SAT_A, LIGHTNESS);
  const b = hslToRgb(h2, SAT_B, LIGHTNESS);
  let worst = Infinity;
  for (let i = 0; i <= 10; i += 1) {
    const t = i / 10;
    const c = [
      a[0] + (b[0] - a[0]) * t,
      a[1] + (b[1] - a[1]) * t,
      a[2] + (b[2] - a[2]) * t,
    ];
    worst = Math.min(worst, contrast(INK, c));
  }
  return worst;
}

const table = [];
let worst = { c: Infinity, h: -1 };
for (let h = 0; h < 360; h += 1) {
  const c = tileWorst(h);
  if (h % 30 === 0) table.push([h, c]);
  if (c < worst.c) worst = { c, h };
}

console.log('hue  min contrast on tile');
for (const [h, c] of table) console.log(`${String(h).padStart(3)}  ${c.toFixed(2)}:1`);
console.log(`\nworst hue: h=${worst.h} -> ${worst.c.toFixed(2)}:1`);
console.log(worst.c >= 4.5 ? 'PASS: every hue clears 4.5:1 across the whole tile' : 'FAIL: raise LIGHTNESS or change the ink');
process.exit(worst.c >= 4.5 ? 0 : 1);

#!/usr/bin/env node
/**
 * Dev-only CDP driver for the Tauri (WebView2) client.
 *
 * The WebView2 runtime exposes Chrome DevTools Protocol when the app is started
 * with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`.
 * That lets us verify and screenshot the real renderer without touching the
 * product code (the Electron build had `SMSGLASS_SHOT` for the same job).
 *
 *   node scripts/cdp.cjs eval "document.title"
 *   node scripts/cdp.cjs eval "window.sms.getState().then(s => s.messages.length)"
 *   node scripts/cdp.cjs shot "H:/tmp/shot.png"
 *   node scripts/cdp.cjs click "#btnTheme"
 *   node scripts/cdp.cjs wait 1200
 *
 * Exit codes: 0 ok, 1 failure. Prints JSON results for `eval`.
 */
'use strict';

const fs = require('fs');
const path = require('path');

const PORT = Number(process.env.CDP_PORT || 9222);

async function targets() {
  const res = await fetch(`http://127.0.0.1:${PORT}/json/list`);
  if (!res.ok) throw new Error(`CDP list HTTP ${res.status}`);
  return res.json();
}

async function firstPage() {
  const list = await targets();
  const page = list.find((t) => t.type === 'page' && t.webSocketDebuggerUrl);
  if (!page) throw new Error('no page target — is the app running with --remote-debugging-port?');
  return page;
}

function connect(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const pending = new Map();
    let nextId = 1;
    ws.addEventListener('open', () => {
      resolve({
        send(method, params) {
          const id = nextId++;
          ws.send(JSON.stringify({ id, method, params: params || {} }));
          return new Promise((res, rej) => pending.set(id, { res, rej }));
        },
        close() {
          try { ws.close(); } catch (_) {}
        },
      });
    });
    ws.addEventListener('message', (event) => {
      let msg;
      try { msg = JSON.parse(event.data); } catch (_) { return; }
      if (msg.id && pending.has(msg.id)) {
        const { res, rej } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) rej(new Error(JSON.stringify(msg.error)));
        else res(msg.result);
      }
    });
    ws.addEventListener('error', (err) => reject(err.error || new Error('websocket error')));
  });
}

async function evaluate(client, expression) {
  const result = await client.send('Runtime.evaluate', {
    expression,
    awaitPromise: true,
    returnByValue: true,
    userGesture: true,
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text + ' ' + JSON.stringify(result.exceptionDetails.exception && result.exceptionDetails.exception.description || ''));
  }
  return result.result ? result.result.value : undefined;
}

async function screenshot(client, file) {
  await client.send('Page.enable');
  // freeze animations so a screenshot never catches a half-played transition
  await evaluate(client, "(() => { const s = document.createElement('style'); s.id='__cdp_shot'; s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}'; document.head.appendChild(s); void document.body.offsetHeight; })()");
  await new Promise((r) => setTimeout(r, 200));
  const shot = await client.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  fs.mkdirSync(path.dirname(path.resolve(file)), { recursive: true });
  fs.writeFileSync(path.resolve(file), Buffer.from(shot.data, 'base64'));
  return path.resolve(file);
}

async function main() {
  const [command, ...rest] = process.argv.slice(2);
  const page = await firstPage();
  const client = await connect(page.webSocketDebuggerUrl);
  try {
    switch (command) {
      case 'eval': {
        const value = await evaluate(client, rest.join(' '));
        console.log(JSON.stringify(value, null, 2));
        return 0;
      }
      case 'click': {
        const selector = rest[0];
        const ok = await evaluate(client, `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false; el.click(); return true; })()`);
        if (!ok) throw new Error(`selector not found: ${selector}`);
        console.log(`clicked ${selector}`);
        return 0;
      }
      case 'set': {
        const selector = rest[0];
        const value = rest.slice(1).join(' ');
        const ok = await evaluate(client, `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false; el.value = ${JSON.stringify(value)}; el.dispatchEvent(new Event('input', {bubbles:true})); el.dispatchEvent(new Event('change', {bubbles:true})); return true; })()`);
        if (!ok) throw new Error(`selector not found: ${selector}`);
        console.log(`set ${selector} = ${value}`);
        return 0;
      }
      case 'shot': {
        const file = await screenshot(client, rest[0]);
        console.log(file);
        return 0;
      }
      case 'wait': {
        await new Promise((r) => setTimeout(r, Number(rest[0] || 500)));
        return 0;
      }
      case 'targets': {
        console.log(JSON.stringify(await targets(), null, 2));
        return 0;
      }
      default:
        throw new Error(`unknown command: ${command}`);
    }
  } finally {
    client.close();
  }
}

main()
  .then((code) => process.exit(code || 0))
  .catch((err) => {
    console.error(`[cdp] ${err.message}`);
    process.exit(1);
  });

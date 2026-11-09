'use strict';
/**
 * SmsBridge client — talks to the phone's local API.
 *
 * Transport (SmsBridge 1.0.1+): the phone speaks HTTPS only, with a
 * self-signed certificate. There is no CA for a LAN IP, so we trust on first
 * use: the first connection fetches the certificate, we keep the PEM on the
 * phone record, and every later request is verified against that exact cert.
 * A swapped certificate is a hard error, never a silent downgrade.
 *
 * Signing (see hermes-android-sms/API.md):
 *   canonical = METHOD \n PATH \n RAW_QUERY_STRING \n TIMESTAMP_MS \n NONCE
 *   signature = hex(HMAC-SHA256(token, canonical))
 *
 * The phone lives on the LAN, so requests go straight out through node:https —
 * never through a system proxy (Clash & friends answer 502 for LAN hosts).
 */

const http = require('node:http');
const crypto = require('node:crypto');
const { t } = require('../shared/i18n');

class BridgeError extends Error {
  constructor(message, { status = 0, code = 'error', payload = null } = {}) {
    super(message);
    this.name = 'BridgeError';
    this.status = status;
    this.code = code;
    this.payload = payload;
  }
}

function normalizeBase(url) {
  let u = String(url || '').trim();
  if (!u) throw new BridgeError(t('请填写手机地址'));
  // 1.0.1+ dropped plain HTTP entirely; rewriting keeps older entries usable.
  u = u.replace(/^http:\/\//i, 'https://');
  if (!/^https?:\/\//i.test(u)) u = 'https://' + u;
  u = u.replace(/\/+$/, '');
  let parsed;
  try {
    parsed = new URL(u);
  } catch {
    throw new BridgeError(t('地址格式不正确，例如 https://192.168.1.5:8787'));
  }
  if (!parsed.port) parsed.port = '8787';
  return `${parsed.protocol}//${parsed.hostname}:${parsed.port}`;
}

function sha256hex(buf) {
  return crypto.createHash('sha256').update(buf).digest('hex');
}

/** DER -> PEM, so we can render the certificate as a file-free string. */
function pemFromDer(der) {
  const lines = Buffer.from(der).toString('base64').match(/.{1,64}/g).join('\n');
  return `-----BEGIN CERTIFICATE-----\n${lines}\n-----END CERTIFICATE-----\n`;
}

/** SHA-256 of a pinned PEM, lowercase hex. */
function fingerprintOfPem(pem) {
  return sha256hex(new crypto.X509Certificate(pem).raw);
}

/** Human form, e.g. "1D 96 B0 E7 ...", same as the phone's pairing screen. */
function prettyFingerprint(hex) {
  const parts = String(hex || '').toUpperCase().match(/.{1,2}/g);
  return parts ? parts.join(' ') : '';
}

function encodeQuery(params) {
  const usp = new URLSearchParams();
  for (const [k, v] of Object.entries(params || {})) {
    if (v === undefined || v === null || v === '') continue;
    usp.append(k, String(v));
  }
  return usp.toString();
}

function rawRequest({
  baseUrl, method = 'GET', path, query = '', headers = {}, body = null,
  timeout = 12000, certPem = null, trustAny = false,
}) {
  const url = new URL(baseUrl);
  const isHttps = url.protocol === 'https:';
  const lib = isHttps ? require('node:https') : http;

  const payload = body == null ? null : Buffer.from(
    typeof body === 'string' ? body : JSON.stringify(body), 'utf8'
  );
  const hdrs = { Accept: 'application/json', ...headers };
  if (payload) {
    // NanoHTTPD picks the body charset from this header and falls back to
    // US-ASCII when it is missing, which turns a Chinese client name into U+FFFD.
    hdrs['Content-Type'] = 'application/json; charset=utf-8';
    hdrs['Content-Length'] = String(payload.length);
  }

  const options = {
    protocol: url.protocol,
    hostname: url.hostname,
    port: url.port || (isHttps ? 443 : 80),
    method,
    path: path + (query ? '?' + query : ''),
    headers: hdrs,
    timeout,
    // LAN only: do not inherit any proxy agent
  };
  if (isHttps) {
    // The certificate is self-signed and issued for a bare IP, so hostname
    // matching is meaningless — we verify by pinning the exact certificate.
    options.checkServerIdentity = () => undefined;
    if (certPem) {
      options.ca = [certPem];
      options.rejectUnauthorized = true;
    } else {
      options.rejectUnauthorized = Boolean(trustAny);
    }
  }

  return new Promise((resolve, reject) => {
    const req = lib.request(options, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let json = null;
        try { json = text ? JSON.parse(text) : null; } catch { /* not json */ }
        resolve({ status: res.statusCode || 0, json, text });
      });
    });

    req.on('timeout', () => {
      req.destroy(new BridgeError(t('手机没有响应（超时）。确认同一 Wi-Fi、App 服务已启动、省电模式已关闭'), { code: 'timeout' }));
    });
    req.on('error', (err) => {
      if (err instanceof BridgeError) return reject(err);
      const tlsHints = {
        DEPTH_ZERO_SELF_SIGNED_CERT: '这台手机的证书还没有被信任：请重新配对，配对时会记录并固定它的证书',
        SELF_SIGNED_CERT_IN_CHAIN: '这台手机的证书还没有被信任：请重新配对，配对时会记录并固定它的证书',
        UNABLE_TO_VERIFY_LEAF_SIGNATURE: '手机证书校验失败：可能重装过 App，请重新配对',
        CERT_HAS_EXPIRED: '手机证书已过期：请在手机上更新 App 后重新配对',
        ERR_TLS_CERT_ALTNAME_INVALID: '手机证书与地址不匹配，请重新配对',
        EPROTO: 'TLS 握手失败：手机端可能还是只支持 HTTP 的旧版本，请更新 App',
      };
      const hint = tlsHints[err.code]
        || (err.code === 'ECONNREFUSED'
          ? t('连接被拒绝：手机上的短信桥服务可能未启动')
          : err.code === 'EHOSTUNREACH' || err.code === 'ENETUNREACH'
            ? t('网络不可达：请确认电脑与手机在同一局域网')
            : err.code === 'ECONNRESET'
              ? t('连接被重置：手机端可能被系统冻结')
              : err.message);
      reject(new BridgeError(hint, { code: err.code || 'network' }));
    });
    if (payload) req.write(payload);
    req.end();
  });
}

function fail(result, fallback) {
  const code = result.json?.error?.code || (result.status === 0 ? 'network' : 'http_' + result.status);
  const msg = result.json?.error?.message || fallback || `HTTP ${result.status}`;
  return new BridgeError(msg, { status: result.status, code, payload: result.json });
}

/**
 * TOFU: fetch the certificate the phone presents, without trusting it yet.
 * The caller decides what to do with it — pairing stores it on the phone record
 * so every later request is verified against exactly this certificate.
 */
async function pinCertificate(baseUrl, timeout = 10000) {
  const base = normalizeBase(baseUrl);
  const target = new URL(base);
  const https = require('node:https');

  return new Promise((resolve, reject) => {
    const req = https.request({
      hostname: target.hostname,
      port: target.port || 443,
      path: '/v1/health',
      method: 'GET',
      headers: { Accept: 'application/json' },
      rejectUnauthorized: false,
      checkServerIdentity: () => undefined,
      timeout,
      agent: false,
    }, (res) => {
      const cert = res.socket.getPeerCertificate(true);
      res.resume();
      if (!cert || !cert.raw) {
        reject(new BridgeError(t('手机没有提供 TLS 证书')));
        return;
      }
      const pem = pemFromDer(cert.raw);
      const fingerprint = sha256hex(cert.raw);
      resolve({ base, pem, fingerprint, pretty: prettyFingerprint(fingerprint) });
    });

    req.on('timeout', () => {
      req.destroy(new BridgeError(t('手机没有响应（超时）。确认同一 Wi-Fi、App 服务已启动、省电模式已关闭'), { code: 'timeout' }));
    });
    req.on('error', (err) => {
      if (err instanceof BridgeError) { reject(err); return; }
      if (err.code === 'EPROTO') {
        reject(new BridgeError(t('手机端的服务不是 HTTPS：请把手机 App 更新到 1.0.1 及以上'), { code: 'protocol' }));
        return;
      }
      const hint = err.code === 'ECONNREFUSED'
        ? t('连接被拒绝：手机上的短信桥服务可能未启动')
        : err.code === 'EHOSTUNREACH' || err.code === 'ENETUNREACH'
          ? t('网络不可达：请确认电脑与手机在同一局域网')
          : err.message;
      reject(new BridgeError(hint, { code: err.code || 'network' }));
    });
    req.end();
  });
}

async function health(baseUrl, opts = {}) {
  const o = typeof opts === 'number' ? { timeout: opts } : (opts || {});
  const base = normalizeBase(baseUrl);
  const result = await rawRequest({
    baseUrl: base,
    path: '/v1/health',
    timeout: o.timeout || 6000,
    certPem: o.certPem || null,
    trustAny: Boolean(o.trustAny),
  });
  if (result.status !== 200 || !result.json?.ok) {
    throw fail(result, t('手机端健康检查失败'));
  }
  return result.json;
}

async function pair({ baseUrl, code, name, certPem }) {
  const base = normalizeBase(baseUrl);
  const result = await rawRequest({
    baseUrl: base,
    method: 'POST',
    path: '/v1/pair',
    body: { code: String(code || '').trim(), name: name || 'Windows SMS Bridge' },
    timeout: 12000,
    certPem: certPem || null,
  });
  if (result.status !== 200 || !result.json?.ok) {
    const e = fail(result, t('配对失败'));
    if (e.status === 401) e.message = t('配对码错误或已过期（5 分钟有效、一次性）');
    throw e;
  }
  const { client_id: clientId, token, client_name: clientName } = result.json;
  if (!clientId || !token) throw new BridgeError(t('配对响应缺少凭据'));
  // The phone echoes the name back; older builds decode the body as US-ASCII
  // and return U+FFFD, so prefer our own copy when the echo looks broken.
  const echoed = typeof clientName === 'string' && !clientName.includes('�') ? clientName : '';
  return {
    baseUrl: base,
    clientId,
    token,
    clientName: echoed || name,
    certPem: certPem || null,
    fingerprint: certPem ? fingerprintOfPem(certPem) : '',
  };
}

class BridgeClient {
  constructor({ baseUrl, clientId, token, certPem }, { timeout = 12000 } = {}) {
    this.baseUrl = normalizeBase(baseUrl);
    this.clientId = clientId;
    this.token = token;
    this.certPem = certPem || null;
    this.timeout = timeout;
  }

  signHeaders(method, path, query) {
    const ts = String(Date.now());
    const nonce = crypto.randomBytes(12).toString('hex');
    const canonical = `${method}\n${path}\n${query}\n${ts}\n${nonce}`;
    const signature = crypto.createHmac('sha256', this.token).update(canonical, 'utf8').digest('hex');
    return {
      'X-Client-Id': this.clientId,
      'X-Timestamp': ts,
      'X-Nonce': nonce,
      'X-Signature': signature,
    };
  }

  async call(method, path, params = null, body = null) {
    const query = params ? encodeQuery(params) : '';
    const headers = this.signHeaders(method, path, query);
    const result = await rawRequest({
      baseUrl: this.baseUrl, method, path, query, headers, body,
      timeout: this.timeout, certPem: this.certPem,
    });
    if (result.status !== 200) throw fail(result, t('请求失败 ({path})', { path }));
    if (result.json && result.json.ok === false) throw fail(result, t('手机端返回错误'));
    return result.json;
  }

  whoami() { return this.call('GET', '/v1/whoami'); }

  sms(opts = {}) {
    const { days, address, slot, box = 'inbox', limit = 100, offset = 0, sinceId, q, purpose } = opts;
    const params = { box, limit };
    if (days != null) params.days = days;
    if (address) params.address = address;
    if (slot != null) params.slot = slot;
    if (offset) params.offset = offset;
    if (sinceId != null) params.since_id = sinceId;
    if (q) params.q = q;
    if (purpose) params.purpose = purpose;
    return this.call('GET', '/v1/sms', params);
  }

  events(sinceSeq = 0) { return this.call('GET', '/v1/events', { since_seq: sinceSeq }); }

  stats(days = 7) { return this.call('GET', '/v1/stats', { days }); }

  revokeSelf() { return this.call('POST', '/v1/revoke-self'); }
}

module.exports = {
  BridgeClient,
  BridgeError,
  pair,
  health,
  pinCertificate,
  fingerprintOfPem,
  prettyFingerprint,
  normalizeBase,
};

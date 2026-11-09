# SmsBridge API

Base URL: `https://<phone-ip>:8787`

Transport is **TLS with a self-signed certificate**. The server does not speak
plain HTTP at all.

There is no CA for a LAN IP, so clients trust the device on first use (TOFU): the
first `pair` call fetches the certificate, stores it locally, and every later
request is verified against it. The SHA-256 fingerprint is shown on the phone's
pairing screen for manual comparison, and `pair --fingerprint <hex>` forces the
check.

## 鉴权

配对后得到 `client_id` 和 `token`（32 字节随机，只显示一次）。

除 `/v1/pair` 和 `/v1/health` 外，所有请求必须带：

```
X-Client-Id: <client_id>
X-Timestamp: <epoch milliseconds>
X-Nonce:     <random string, >= 16 chars>
X-Signature: <hex(HMAC-SHA256(token, canonical))>
```

canonical string：

```
METHOD \n PATH \n RAW_QUERY_STRING \n TIMESTAMP \n NONCE
```

- `PATH` 不含域名和 query，例如 `/v1/sms`
- `RAW_QUERY_STRING` 是 URL 里 `?` 之后的原始串，例如 `days=3&limit=20`
- 时间窗 ±5 分钟
- 同一 nonce 5 分钟内不能重复（防重放）
- 签名不匹配返回 401

## 端点

### `POST /v1/pair`（免签）

```json
{"code": "123456", "name": "DSH-PC"}
```

响应：

```json
{
  "ok": true,
  "client_id": "uuid",
  "client_name": "DSH-PC",
  "token": "base64url-43-chars",
  "note": "store the token on the agent side; the phone never shows it again"
}
```

6 位码在手机上生成，5 分钟有效，一次性，每分钟最多尝试 20 次。

### `GET /v1/health`（免签）

```json
{
  "ok": true,
  "app": "SmsBridge", "version": "1.0.0",
  "android": "16", "sdk": 36, "model": "Pixel 9",
  "battery": 87, "charging": false, "uptime_s": 1234,
  "paired_clients": 1, "pending_events": 3, "latest_seq": 12,
  "server_time": 1762345678901,
  "sims": [{"slot": 0, "sub_id": 1, "carrier": "中国电信", "number": "+8613800000000"},
           {"slot": 1, "sub_id": 2, "carrier": "中国电信", "number": ""}]
}
```

`number` 是这张卡的本机号码，读自系统的 `SubscriptionInfo`。运营商没有把号码写进 SIM 卡时为空字符串（部分机型/号段），客户端应回退到 `slot` 分组，不要假设它一定有值。

### `GET /v1/sms`（签名）

| 参数 | 说明 |
|---|---|
| `days` | 最近多少天（服务端会钳到手机设置的上限） |
| `from` / `to` | 可选，epoch ms，优先于 days |
| `box` | `inbox`（默认）/ `sent` / `draft` / `all` |
| `address` | 号码或前缀，多个用逗号分隔，例如 `10086,95588` |
| `slot` | SIM 卡槽 0 或 1，不传 = 全部 |
| `limit` | 条数上限，默认 50（服务端钳到设置值） |
| `offset` | 分页偏移 |
| `since_id` | 只要 `_id` 大于它的（增量） |
| `q` | 正文关键词（LIKE） |
| `purpose` | `otp` 时启用验证码模式 |

响应：

```json
{
  "ok": true,
  "query": {"days": 3, "box": "inbox", "address": "10086", "slot": null,
            "limit": 20, "offset": 0, "purpose": "full"},
  "count": 2,
  "messages": [
    {
      "id": 12345,
      "address": "10086",
      "date": 1762345600000,
      "date_iso": "2026-11-04 22:46:40",
      "type": "inbox",
      "slot": 0,
      "sub_id": 1,
      "carrier": "中国电信",
      "sim_number": "+8613800000000",
      "read": true,
      "body": "【中国电信】您的验证码是 836291，5 分钟内有效。"
    }
  ]
}
```

`purpose=otp` 时每条消息换成：

```json
{
  "id": 12345, "address": "10086", "date": 1762345600000,
  "date_iso": "2026-11-04 22:46:40", "type": "inbox", "slot": 0,
  "sub_id": 1, "carrier": "中国电信", "sim_number": "+8613800000000", "read": true,
  "otp": "836291",
  "matched_by": "验证码",
  "masked_body": "【中国电信】您的验证码是 836291，* 分钟内有效。"
}
```

没有提取到验证码的消息会被过滤掉。

### `GET /v1/events`（签名）

| 参数 | 说明 |
|---|---|
| `since_seq` | 只要 seq 大于它的，默认 0 |

```json
{
  "ok": true,
  "since_seq": 0,
  "latest_seq": 12,
  "count": 2,
  "events": [
    {"seq": 11, "sms_id": 12344, "address": "10086", "date": 1762345600000,
     "date_iso": "2026-11-04 22:46:40", "type": "inbox", "sub_id": 1,
     "preview": "【中国电信】您的验证码是 836291…"}
  ]
}
```

队列最多保留最近 500 条事件。

### `GET /v1/stats`（签名）

| 参数 | 说明 |
|---|---|
| `days` | 统计窗口，默认 7 |

按发送方聚合条数。

### `GET /v1/whoami`（签名）

当前客户端身份、创建时间、请求次数、最后活跃。

### `POST /v1/revoke-self`（签名）

吊销自己的 token，立即失效。

## 错误格式

```json
{"ok": false, "error": {"code": "unauthorized", "message": "bad signature"}}
```

| HTTP | code | 含义 |
|---|---|---|
| 401 | `unauthorized` | 缺签名 / 签名错 / token 已吊销 |
| 401 | `pair_failed` | 配对码错误或过期 |
| 403 | `otp_mode_disabled` | 手机端关闭了 OTP 模式 |
| 404 | `not_found` | 端点不存在 |
| 429 | `rate_limited` | 超过手机设置的每分钟请求数 |
| 500 | `internal_error` | 手机端异常 |

"""Generate throw-away demo data (two phones, several conversations) for UI review."""
import json
import os
import random
import time

demo = '.demo-data'
os.makedirs(demo, exist_ok=True)
now = int(time.time() * 1000)
day = 86400000

phones = [
    {
        "id": "demo-phone-1", "name": "主力机 · Pixel 9", "url": "http://192.168.1.5:8787",
        "clientId": "demo-client", "clientName": "Windows SMS Bridge",
        "createdAt": now - 5 * day, "lastSyncAt": now - 120000,
        "lastSmsId": 999, "lastSeq": 0, "tokenEnc": "", "tokenRaw": "demo-token",
        "health": {"ok": True, "model": "Pixel 9", "android": "16", "battery": 76, "charging": False,
                   "sims": [{"slot": 0, "carrier": "中国电信"}], "fetchedAt": now - 60000},
    },
    {
        "id": "demo-phone-2", "name": "备用机 · 小米 14", "url": "http://192.168.1.9:8787",
        "clientId": "demo-client-2", "clientName": "Windows SMS Bridge",
        "createdAt": now - 9 * day, "lastSyncAt": now - 3600000,
        "lastSmsId": 88, "lastSeq": 0, "tokenEnc": "", "tokenRaw": "demo-token-2",
        "health": {"ok": True, "model": "23127PN0CC", "android": "15", "battery": 41, "charging": True,
                   "sims": [{"slot": 1, "carrier": "中国移动"}], "fetchedAt": now - 240000},
    },
]

convos = [
    ("10086", "中国移动", 0, [
        (3, "inbox", "【中国移动】您本月已使用流量 12.4GB，剩余 7.6GB。回复 CXLL 查询详情。"),
        (2, "inbox", "【中国移动】尊敬的客户，您的话费余额为 68.20 元，本期账单已出账。"),
    ]),
    ("95588", "工商银行", 0, [
        (1, "inbox", "【工商银行】您尾号 8823 的账户于 11 月 8 日 14:32 转入 12,800.00 元，余额 45,209.61 元。"),
        (1, "sent", "收到，谢谢"),
    ]),
    ("106903", "京东", 0, [
        (5, "inbox", "【京东】您的验证码是 836291，5 分钟内有效。请勿泄露给他人。"),
        (4, "inbox", "【京东】您购买的商品已发货，快递单号 JD8842910332，预计明日送达。"),
    ]),
    ("10000", "中国电信", 0, [
        (4, "inbox", "尊敬的客户您好！您被评为中国电信2星级客户，有效期为12个月。自下月起，我们将为您提供消费定级，详细请登录中国电信APP ( a.189.cn/jiYn6Q ) 或关注“湖南电信”微信公众号等方式了解体验。"),
    ]),
    ("+8613800138000", "张伟", 0, [
        (0, "inbox", "晚上七点老地方，别迟到啊"),
        (0, "sent", "好，我六点半出发"),
        (1, "inbox", "记得带上那个硬盘"),
    ]),
    ("12306", "铁路12306", 1, [
        (2, "inbox", "【铁路12306】订单 E8842910，11月10日 G6013 次 长沙南-深圳北 08:32 开，二等座 07 车 12F。"),
    ]),
]

mails = []
mid = 900
for addr, carrier, phone_idx, items in convos:
    for days_ago, box, body in items:
        mid += 1
        ts = now - days_ago * day - random.randint(0, 6) * 3600000 + random.randint(0, 59) * 60000
        mails.append({
            "id": mid, "address": addr, "body": body, "date": ts, "type": box,
            "slot": 0, "subId": 1, "carrier": carrier,
            "read": box == "sent" or days_ago > 3,
            "seen": box == "sent" or days_ago > 3,
        })

mails.sort(key=lambda m: -m["date"])
cfg = {
    "version": 1,
    "settings": {"autoStart": False, "startHidden": False, "syncIntervalMin": 15, "syncOnStart": False,
                 "notifyNew": True, "pullDays": 30, "keepDays": 0, "closeToTray": True, "theme": "dark"},
    "phones": phones,
}
json.dump(cfg, open(os.path.join(demo, 'config.json'), 'w', encoding='utf-8'), ensure_ascii=False, indent=2)
json.dump({"version": 1, "messages": {"demo-phone-1": mails, "demo-phone-2": []}},
          open(os.path.join(demo, 'messages.json'), 'w', encoding='utf-8'), ensure_ascii=False, indent=1)
print('demo data:', len(mails), 'messages')

#!/usr/bin/env python3
"""smsctl — command line client for the SmsBridge android app.

Examples:
    python smsctl.py pair --url http://192.168.1.100:8787 --code 123456 --name my-agent
    python smsctl.py health
    python smsctl.py search --days 3 --address 10086 --limit 20
    python smsctl.py otp --address 10086 --days 1
    python smsctl.py events --since 0
"""

from __future__ import annotations

import argparse
import json
import sys

from smsbridge_client import (
    DEFAULT_URL,
    Credentials,
    SmsBridge,
    SmsBridgeError,
    pair,
)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")


def dump(obj) -> None:
    print(json.dumps(obj, ensure_ascii=False, indent=2))


def fmt_ts(ms: int | None) -> str:
    if not ms:
        return "-"
    import datetime
    return datetime.datetime.fromtimestamp(ms / 1000).strftime("%Y-%m-%d %H:%M:%S")


def print_messages(resp: dict, otp_mode: bool) -> None:
    msgs = resp.get("messages", [])
    print(f"共 {len(msgs)} 条")
    for m in msgs:
        slot = m.get("slot")
        carrier = m.get("carrier") or ""
        tag = f"[卡{slot + 1}]" if isinstance(slot, int) and slot >= 0 else "[卡?]"
        if otp_mode:
            print(f"{m.get('date_iso')} {tag} {m.get('address')} 验证码={m.get('otp')}")
            print(f"    {m.get('masked_body')}")
        else:
            print(f"{m.get('date_iso')} {tag} {m.get('address')} {carrier}")
            print(f"    {m.get('body')}")


def get_client() -> SmsBridge:
    return SmsBridge(Credentials.load())


def main() -> int:
    ap = argparse.ArgumentParser(prog="smsctl", description="SmsBridge CLI")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("pair", help="pair this agent with the phone")
    p.add_argument("--url", default=DEFAULT_URL)
    p.add_argument("--code", required=True, help="6-digit code shown on the phone")
    p.add_argument("--name", default="agent")

    sub.add_parser("health", help="bridge / battery / sims")

    p = sub.add_parser("whoami", help="show paired client identity")
    p.set_defaults()

    p = sub.add_parser("search", help="search sms")
    p.add_argument("--days", type=int)
    p.add_argument("--address")
    p.add_argument("--slot", type=int)
    p.add_argument("--box", default="inbox", choices=["inbox", "sent", "draft", "all"])
    p.add_argument("--limit", type=int, default=20)
    p.add_argument("--q")
    p.add_argument("--json", action="store_true")

    p = sub.add_parser("otp", help="recent verification codes only (body masked)")
    p.add_argument("--address")
    p.add_argument("--days", type=int, default=1)
    p.add_argument("--slot", type=int)
    p.add_argument("--limit", type=int, default=10)
    p.add_argument("--json", action="store_true")

    p = sub.add_parser("events", help="incremental new-sms events")
    p.add_argument("--since", type=int, default=0)
    p.add_argument("--json", action="store_true")

    p = sub.add_parser("stats", help="count by address")
    p.add_argument("--days", type=int, default=7)
    p.add_argument("--json", action="store_true")

    args = ap.parse_args()

    try:
        if args.cmd == "pair":
            if not args.url:
                print("ERROR: --url is required, e.g. --url http://192.168.1.100:8787",
                      file=sys.stderr)
                return 2
            creds = pair(args.url, args.code, args.name)
            print(f"paired: client_id={creds.client_id}")
            print(f"config saved to .smsbridge.json (base_url={creds.base_url})")
            return 0

        client = get_client()

        if args.cmd == "health":
            dump(client.health())

        elif args.cmd == "whoami":
            dump(client.whoami())

        elif args.cmd == "search":
            resp = client.sms(
                days=args.days, address=args.address, slot=args.slot,
                box=args.box, limit=args.limit, q=args.q,
            )
            if args.json:
                dump(resp)
            else:
                print_messages(resp, otp_mode=False)

        elif args.cmd == "otp":
            resp = client.otp(address=args.address, days=args.days,
                              slot=args.slot, limit=args.limit)
            if args.json:
                dump(resp)
            else:
                print_messages(resp, otp_mode=True)

        elif args.cmd == "events":
            resp = client.events(since_seq=args.since)
            if args.json:
                dump(resp)
            else:
                print(f"since={args.since} latest={resp.get('latest_seq')} "
                      f"count={resp.get('count')}")
                for e in resp.get("events", []):
                    print(f"seq={e['seq']} {fmt_ts(e['date'])} {e['address']} {e['preview']}")

        elif args.cmd == "stats":
            resp = client.stats(days=args.days)
            if args.json:
                dump(resp)
            else:
                print(f"最近 {resp.get('days')} 天共 {resp.get('total')} 条")
                for a in resp.get("addresses", []):
                    print(f"  {a['address']:<20} {a['count']:>5} 条  最后 {a['last_date_iso']}")

        return 0

    except SmsBridgeError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        if e.payload:
            print(json.dumps(e.payload, ensure_ascii=False, indent=2), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

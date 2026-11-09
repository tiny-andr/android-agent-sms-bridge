#!/usr/bin/env python3
"""MCP server exposing a paired SmsBridge phone to any MCP capable agent.

Run:  python sms_mcp_server.py
Pair first with:  python smsctl.py pair --url https://192.168.1.100:8787 --code 123456
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from mcp.server.mcpserver import MCPServer  # mcp >= 2.0

from smsbridge_client import (
    Credentials,
    SmsBridge,
    SmsBridgeError,
    pair as do_pair,
)

mcp = MCPServer(
    "smsbridge",
    title="SmsBridge",
    instructions=(
        "Read SMS from a paired android phone. Prefer sms_otp for verification codes: "
        "it returns only the code and a masked body, so no full message content leaks."
    ),
)


def _client() -> SmsBridge:
    return SmsBridge(Credentials.load())


def _dump(obj) -> str:
    return json.dumps(obj, ensure_ascii=False, indent=2)


@mcp.tool()
def sms_health() -> str:
    """Report bridge status, battery, and the two SIM slots of the phone."""
    try:
        return _dump(_client().health())
    except SmsBridgeError as e:
        return _dump({"ok": False, "error": str(e)})


@mcp.tool()
def sms_search(
    days: int = 3,
    address: str = "",
    slot: int = -1,
    box: str = "inbox",
    limit: int = 20,
    q: str = "",
) -> str:
    """Search SMS by time range, sender address and SIM slot.

    Args:
        days: how many days back to search (the phone may cap this).
        address: sender number or prefix, e.g. "10086"; empty = any sender.
        slot: SIM slot index 0 or 1; -1 = both.
        box: inbox | sent | all.
        limit: max messages to return.
        q: optional text keyword inside the body.
    """
    try:
        resp = _client().sms(
            days=days,
            address=address or None,
            slot=None if slot < 0 else slot,
            box=box,
            limit=limit,
            q=q or None,
        )
        return _dump(resp)
    except SmsBridgeError as e:
        return _dump({"ok": False, "error": str(e)})


@mcp.tool()
def sms_latest(count: int = 10, box: str = "inbox") -> str:
    """Return the newest SMS messages."""
    try:
        return _dump(_client().sms(days=7, box=box, limit=count))
    except SmsBridgeError as e:
        return _dump({"ok": False, "error": str(e)})


@mcp.tool()
def sms_otp(address: str = "", days: int = 1, slot: int = -1) -> str:
    """Return recent verification codes only, with the message body masked.

    This is the privacy-preserving way to get an OTP for an automated login.
    """
    try:
        return _dump(_client().otp(
            address=address or None,
            days=days,
            slot=None if slot < 0 else slot,
        ))
    except SmsBridgeError as e:
        return _dump({"ok": False, "error": str(e)})


@mcp.tool()
def sms_events(since_seq: int = 0) -> str:
    """Pull new-SMS events after a sequence number (incremental, no duplicates).

    Call once to get baseline, then poll with the returned latest_seq.
    """
    try:
        return _dump(_client().events(since_seq=since_seq))
    except SmsBridgeError as e:
        return _dump({"ok": False, "error": str(e)})


@mcp.tool()
def sms_stats(days: int = 7) -> str:
    """Count messages per sender address over the last N days."""
    try:
        return _dump(_client().stats(days=days))
    except SmsBridgeError as e:
        return _dump({"ok": False, "error": str(e)})


@mcp.tool()
def sms_pair(url: str, code: str, name: str = "mcp-agent") -> str:
    """Pair with the phone using the 6-digit code shown on its Pairing screen.

    Saves credentials to .smsbridge.json next to this script.
    """
    try:
        creds = do_pair(url, code, name)
        return _dump({"ok": True, "client_id": creds.client_id, "base_url": creds.base_url})
    except Exception as e:  # noqa: BLE001
        return _dump({"ok": False, "error": str(e)})


if __name__ == "__main__":
    mcp.run()

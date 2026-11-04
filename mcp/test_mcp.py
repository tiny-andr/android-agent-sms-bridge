import json
import subprocess
import sys
from pathlib import Path

PY = sys.executable
CWD = str(Path(__file__).resolve().parent)

proc = subprocess.Popen(
    [PY, "sms_mcp_server.py"],
    cwd=CWD,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)


def send(obj):
    proc.stdin.write((json.dumps(obj, ensure_ascii=False) + "\n").encode("utf-8"))
    proc.stdin.flush()


def recv(timeout=15):
    line = proc.stdout.readline()
    if not line:
        err = proc.stderr.read().decode("utf-8", "replace")
        raise SystemExit(f"no response, stderr={err}")
    return json.loads(line.decode("utf-8"))


send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {},
    "clientInfo": {"name": "test", "version": "1.0"},
}})
r = recv()
print("initialize ->", json.dumps(r.get("result", {}).get("serverInfo", {}), ensure_ascii=False))

send({"jsonrpc": "2.0", "method": "notifications/initialized"})

send({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
r = recv()
tools = [t["name"] for t in r["result"]["tools"]]
print("tools ->", tools)

send({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
      "params": {"name": "sms_health", "arguments": {}}})
r = recv()
text = r["result"]["content"][0]["text"]
print("sms_health ->", text[:400].replace("\n", " "))

send({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
      "params": {"name": "sms_search", "arguments": {"days": 30, "limit": 3}}})
r = recv()
text = r["result"]["content"][0]["text"]
print("sms_search ->", text[:500].replace("\n", " "))

proc.terminate()
try:
    proc.wait(timeout=5)
except Exception:
    proc.kill()
print("OK")

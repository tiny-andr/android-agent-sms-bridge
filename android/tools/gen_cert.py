#!/usr/bin/env python3
"""Generate the self-signed TLS certificate the app serves.

Cross-platform replacement for tools/gen-cert.ps1 (uses the JDK's keytool).

Writes:
    keystore/server.p12             private key, git-ignored
    app/src/main/assets/server.p12  packaged into the APK, git-ignored
    mcp/smsbridge-cert.pem          client pinning certificate, tracked in git

Running this overwrites mcp/smsbridge-cert.pem. That is intended for a fresh
clone: the certificate in the repo belongs to the official release build, so a
self-built APK must publish its own PEM for its own clients.

Usage:
    python tools/gen_cert.py
    SMSBRIDGE_CERT_PASSWORD=secret python tools/gen_cert.py
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# The repo root (one level above android/) holds the shared mcp/ clients.
REPO = ROOT.parent
KEYSTORE = ROOT / "keystore" / "server.p12"
ASSETS = ROOT / "app" / "src" / "main" / "assets" / "server.p12"
PEM = REPO / "mcp" / "smsbridge-cert.pem"

PASSWORD = os.environ.get("SMSBRIDGE_CERT_PASSWORD", "smsbridge")
ALIAS = "smsbridge"
DNAME = "CN=SMS Bridge,O=tiny-andr,C=CN"
SAN = "SAN=DNS:localhost,IP:127.0.0.1,IP:0.0.0.0"


def find_keytool() -> str | None:
    java_home = os.environ.get("JAVA_HOME")
    if java_home:
        exe = "keytool.exe" if os.name == "nt" else "keytool"
        candidate = Path(java_home) / "bin" / exe
        if candidate.exists():
            return str(candidate)
    return shutil.which("keytool")


def main() -> int:
    keytool = find_keytool()
    if not keytool:
        print("keytool not found. Install a JDK 17+ or set JAVA_HOME.", file=sys.stderr)
        return 1

    KEYSTORE.parent.mkdir(parents=True, exist_ok=True)
    if KEYSTORE.exists():
        KEYSTORE.unlink()

    subprocess.run(
        [
            keytool, "-genkeypair",
            "-alias", ALIAS,
            "-keyalg", "RSA",
            "-keysize", "2048",
            "-validity", "36500",
            "-storetype", "PKCS12",
            "-keystore", str(KEYSTORE),
            "-storepass", PASSWORD,
            "-keypass", PASSWORD,
            "-dname", DNAME,
            "-ext", SAN,
        ],
        check=True,
    )

    ASSETS.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(KEYSTORE, ASSETS)

    PEM.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            keytool, "-exportcert",
            "-alias", ALIAS,
            "-keystore", str(KEYSTORE),
            "-storepass", PASSWORD,
            "-rfc",
            "-file", str(PEM),
        ],
        check=True,
    )

    print(f"wrote {KEYSTORE}")
    print(f"wrote {ASSETS}")
    print(f"wrote {PEM}")
    print()
    print("mcp/smsbridge-cert.pem now matches THIS build's certificate.")
    print("Optional: clients pin it automatically on first pair anyway (TOFU).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

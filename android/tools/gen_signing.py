#!/usr/bin/env python3
"""Generate the release signing keystore for the Android app.

Writes, both git-ignored:
    keystore/release.p12         RSA 2048, 10000 days
    keystore/release.properties  credentials Gradle reads at build time

The signature is what lets an installed copy accept an update in place. If this
file is lost or regenerated, every installed app has to be uninstalled before it
can take a new build, so keep a backup somewhere safe.

Usage:
    python tools/gen_signing.py
    SMSBRIDGE_SIGN_PASSWORD=secret python tools/gen_signing.py
"""

from __future__ import annotations

import os
import secrets
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KEYSTORE = ROOT / "keystore" / "release.p12"
PROPS = ROOT / "keystore" / "release.properties"

ALIAS = "smsbridge"
DNAME = "CN=SMS Bridge,O=tiny-andr,C=CN"
VALIDITY_DAYS = "10000"


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

    if KEYSTORE.exists() and PROPS.exists():
        print(f"{KEYSTORE} already exists - refusing to overwrite.")
        print("Delete both files first if you really want a new signature.")
        return 1

    password = os.environ.get("SMSBRIDGE_SIGN_PASSWORD") or secrets.token_urlsafe(18)

    KEYSTORE.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            keytool, "-genkeypair",
            "-keystore", str(KEYSTORE),
            "-storetype", "PKCS12",
            "-alias", ALIAS,
            "-keyalg", "RSA",
            "-keysize", "2048",
            "-validity", VALIDITY_DAYS,
            "-dname", DNAME,
            "-storepass", password,
            "-keypass", password,
        ],
        check=True,
    )

    PROPS.write_text(
        "# Release signing credentials. Git-ignored - back this up together with\n"
        "# release.p12, otherwise future updates cannot be installed over this one.\n"
        f"storeFile=keystore/release.p12\n"
        f"storePassword={password}\n"
        f"keyAlias={ALIAS}\n"
        f"keyPassword={password}\n",
        encoding="utf-8",
    )

    print(f"keystore : {KEYSTORE}")
    print(f"props    : {PROPS}")
    print("Back both files up now.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

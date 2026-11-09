# Generates the self-signed TLS certificate the app serves.
#
# Produces:
#   keystore/server.p12             private key, git-ignored
#   app/src/main/assets/server.p12  packaged into the APK, git-ignored
#   mcp/smsbridge-cert.pem          client pinning certificate, committed
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\gen-cert.ps1
# Replacing the certificate invalidates mcp\smsbridge-cert.pem pins of existing
# agents; they must copy the new PEM.

param(
    [string]$Password = "smsbridge",
    [string]$Alias = "smsbridge"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

$keytool = $null
if ($env:JAVA_HOME) {
    $candidate = Join-Path $env:JAVA_HOME "bin\keytool.exe"
    if (Test-Path $candidate) { $keytool = $candidate }
}
if (-not $keytool) {
    $cmd = Get-Command keytool -ErrorAction SilentlyContinue
    if ($cmd) { $keytool = $cmd.Source }
}
if (-not $keytool) {
    throw "keytool not found. Install a JDK or set JAVA_HOME."
}

$keystoreDir = Join-Path $root "keystore"
$assetsDir = Join-Path $root "app\src\main\assets"
$p12 = Join-Path $keystoreDir "server.p12"
$pem = Join-Path (Split-Path -Parent $root) "mcp\smsbridge-cert.pem"

New-Item -ItemType Directory -Force -Path $keystoreDir, $assetsDir | Out-Null
if (Test-Path $p12) { Remove-Item $p12 -Force }

& $keytool -genkeypair `
    -alias $Alias `
    -keyalg RSA -keysize 2048 -validity 36500 `
    -storetype PKCS12 -keystore $p12 `
    -storepass $Password -keypass $Password `
    -dname "CN=SMS Bridge,O=tiny-andr,C=CN" `
    -ext "SAN=DNS:localhost,IP:127.0.0.1,IP:0.0.0.0"

Copy-Item $p12 (Join-Path $assetsDir "server.p12") -Force
& $keytool -exportcert -alias $Alias -keystore $p12 -storepass $Password -rfc -file $pem

Write-Host ""
Write-Host "done:"
Write-Host "  $p12"
Write-Host "  $(Join-Path $assetsDir 'server.p12')"
Write-Host "  $pem"

# Builds both clients into dist/.
#
#   .\build.ps1              # both
#   .\build.ps1 -Android     # APK only
#   .\build.ps1 -Windows     # desktop client only
#
# Artifacts: dist\SmsBridge-<version>.apk, dist\Windows-SMS-Bridge-Setup-<version>.exe

param(
    [switch]$Android,
    [switch]$Windows
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$dist = Join-Path $root 'dist'

if (-not $Android -and -not $Windows) { $Android = $true; $Windows = $true }

New-Item -ItemType Directory -Force -Path $dist | Out-Null

if ($Android) {
    $gradle = Join-Path $root 'android\gradlew.bat'
    if (Test-Path $gradle) { $gradleExe = $gradle; $gradleArgs = @('-q') }
    else {
        $gradleExe = 'gradle'
        $gradleArgs = @('-q')
    }
    Write-Host '== Android =='
    & $gradleExe @gradleArgs '-p' (Join-Path $root 'android') 'assembleRelease'
    if ($LASTEXITCODE -ne 0) { throw 'Android build failed' }

    $apk = Get-ChildItem (Join-Path $root 'android\app\build\outputs\apk\release\*.apk') |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    $version = (Select-String -Path (Join-Path $root 'android\app\build.gradle.kts') -Pattern 'versionName\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
    Copy-Item $apk.FullName (Join-Path $dist "SmsBridge-$version.apk") -Force
    Write-Host "  -> dist\SmsBridge-$version.apk"
}

if ($Windows) {
    Write-Host '== Windows =='
    $win = Join-Path $root 'windows-rust'
    $cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
    if (-not $cargo) {
        $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
        if (-not (Test-Path $cargo)) { throw 'cargo not found — install the Rust toolchain first' }
    }
    Push-Location $win
    try {
        if (-not (Test-Path (Join-Path $win 'node_modules'))) { & npm install --no-audit --no-fund }
        if ($LASTEXITCODE -ne 0) { throw 'npm install failed' }
        $env:PATH = (Join-Path $env:USERPROFILE '.cargo\bin') + ';' + $env:PATH
        & npm run dist
        if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
    } finally {
        Pop-Location
    }
    $version = (Get-Content (Join-Path $win 'package.json') -Raw | ConvertFrom-Json).version
    $installer = Join-Path $win "target\release\bundle\nsis\Windows-SMS-Bridge-Setup-$version.exe"
    if (-not (Test-Path $installer)) { throw "installer not found: $installer" }
    Copy-Item $installer (Join-Path $dist "Windows-SMS-Bridge-Setup-$version.exe") -Force
    Write-Host "  -> dist\Windows-SMS-Bridge-Setup-$version.exe"
}

Write-Host 'Done.'

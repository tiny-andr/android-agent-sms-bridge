# Runs the old Electron client and the new Tauri client against the same phone,
# each with a fresh cache, then compares the caches field by field.
#
#   powershell -ExecutionPolicy Bypass -File scripts/parity-sync.ps1 [-WaitSeconds 45]
#
# Both clients get their own copy of the real profile (pairings kept, cached
# messages and sync bookkeeping reset) so each performs a FULL 30-day pull. The
# Electron instance is launched from windows\node_modules (process name
# electron.exe) so it never touches the packaged app the user may be running.

param(
    [int]$WaitSeconds = 45,
    [string]$Root = 'H:\tmp\parity'
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot                    # -> windows-rust
$repoRoot = Split-Path -Parent $repo                        # -> repo root
$electronDir = Join-Path $Root 'electron'
$rustDir = Join-Path $Root 'rust'

Write-Host "[parity] preparing isolated profiles"
& node (Join-Path $PSScriptRoot 'prepare-parity.cjs') (Join-Path $env:APPDATA 'Windows SMS Bridge') $electronDir $rustDir
if ($LASTEXITCODE -ne 0) { throw 'profile preparation failed' }

# ---------------------------------------------------------------- Electron run
Write-Host '[parity] launching the Electron client (fresh full sync)'
$electron = Join-Path $repoRoot 'windows\node_modules\electron\dist\electron.exe'
if (-not (Test-Path $electron)) { throw "electron.exe not found at $electron" }
$env:SMSGLASS_DATA_DIR = $electronDir
Remove-Item Env:\ELECTRON_RUN_AS_NODE -ErrorAction SilentlyContinue
$proc = Start-Process -FilePath $electron -ArgumentList (Join-Path $repoRoot 'windows') -PassThru `
    -RedirectStandardOutput (Join-Path $Root 'electron.out.log') `
    -RedirectStandardError (Join-Path $Root 'electron.err.log')
Start-Sleep -Seconds $WaitSeconds
# stop only our dev instance (the packaged app runs as "Windows SMS Bridge.exe")
Get-CimInstance Win32_Process -Filter "Name='electron.exe'" |
    Where-Object { $_.CommandLine -like '*hermes-android-sms*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch { }
Start-Sleep -Seconds 2
Write-Host "[parity] electron cache: $((Get-Content (Join-Path $electronDir 'messages.json') -Raw).Length) bytes"

# ------------------------------------------------------------------- Tauri run
Write-Host '[parity] launching the Tauri client (fresh full sync)'
$exe = Join-Path $repo 'target\release\windows-sms-bridge.exe'
if (-not (Test-Path $exe)) { $exe = Join-Path $repo 'target\debug\windows-sms-bridge.exe' }
if (-not (Test-Path $exe)) { throw 'Tauri binary not built (cargo build --release)' }
$env:SMSGLASS_DATA_DIR = $rustDir
Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
$proc = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru `
    -RedirectStandardOutput (Join-Path $Root 'rust.out.log') `
    -RedirectStandardError (Join-Path $Root 'rust.err.log')
Start-Sleep -Seconds $WaitSeconds
Get-Process | Where-Object { $_.ProcessName -eq 'windows-sms-bridge' } | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
Write-Host "[parity] tauri cache: $((Get-Content (Join-Path $rustDir 'messages.json') -Raw).Length) bytes"

# --------------------------------------------------------------------- compare
Write-Host '[parity] comparing caches'
& node (Join-Path $PSScriptRoot 'compare-sync.cjs') $electronDir $rustDir
exit $LASTEXITCODE

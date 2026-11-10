#Requires -Version 5.1
<#
.SYNOPSIS
  体积 / 内存测量：旧 Electron 客户端 vs 新 Tauri 客户端。

.DESCRIPTION
  按顺序测量两个应用（先旧后新），输出一张 markdown 表到控制台和
  windows-rust\.shots\measure.md：

    · 安装包大小（NSIS 安装器文件）
    · 解包目录大小（Electron: dist\win-unpacked；Tauri: target\release 顶层文件）
    · 安装后占用（已安装目录的 4 KiB 簇分配大小；未安装时用 7-Zip/Bandizip 解包估算）
    · 空闲 N 秒后、同步后：主进程 Private Working Set 峰值/稳定 + Working Set 稳定

  安全约定：
    · 只结束本脚本自己启动的进程树（taskkill /PID <pid> /T），绝不结束别的进程；
    · 若应用已在运行，只做只读采样并提示，不会重启/结束它；
    · 可选的 %APPDATA%\Windows SMS Bridge 备份/恢复只在"干净运行"（两个应用都没在运行）时进行。

.PARAMETER OldRoot
  旧 Electron 客户端目录，默认 ..\..\windows（相对本脚本）。

.PARAMETER NewRoot
  新 Tauri crate 目录，默认本脚本的上一级（windows-rust）。

.PARAMETER IdleSeconds
  启动后等待"窗口空闲"的秒数，默认 60。

.PARAMETER SampleSeconds
  每个测量窗口的采样秒数（1 s 一次），默认 10。

.PARAMETER SyncTimeoutSeconds
  等待同步完成（%APPDATA% 数据文件发生变化）的超时，默认 120。

.PARAMETER SyncCommand
  可选：触发同步的外部命令（例如 UI 自动化脚本）。不提供时，脚本通过
  "重启应用"触发启动同步（syncOnStart），这是两个客户端都有的行为。

.PARAMETER SkipMemory
  只测体积，不启动应用。

.PARAMETER SkipInstallSize
  跳过"安装后占用"（不做任何解包/注册表测量）。

.PARAMETER SkipDataBackup
  不备份/恢复 %APPDATA%\Windows SMS Bridge。

.PARAMETER RequireStopped
  若目标应用已在运行则直接报错退出（用于获取干净数据）。

.EXAMPLE
  pwsh -File scripts\measure.ps1
  pwsh -File scripts\measure.ps1 -IdleSeconds 60 -SampleSeconds 10
  pwsh -File scripts\measure.ps1 -SkipMemory          # 只测体积
#>
[CmdletBinding()]
param(
    [string]$OldRoot = '',
    [string]$NewRoot = '',
    [int]$IdleSeconds = 60,
    [int]$SampleSeconds = 10,
    [int]$SyncTimeoutSeconds = 120,
    [string]$SyncCommand = '',
    [switch]$SkipMemory,
    [switch]$SkipInstallSize,
    [switch]$SkipDataBackup,
    [switch]$RequireStopped
)

$ErrorActionPreference = 'Stop'
$ProductName = 'Windows SMS Bridge'
$ClusterSize = 4096
$DataDir = Join-Path $env:APPDATA $ProductName

# 脚本本身在 windows-rust\scripts\ 下。
$Here = $PSScriptRoot
if (-not $NewRoot) { $NewRoot = Split-Path -Parent $Here }
if (-not $OldRoot) { $OldRoot = Join-Path (Split-Path -Parent $NewRoot) 'windows' }

$OldRoot = [System.IO.Path]::GetFullPath($OldRoot)
$NewRoot = [System.IO.Path]::GetFullPath($NewRoot)
$OutDir = Join-Path $NewRoot '.shots'
$OutFile = Join-Path $OutDir 'measure.md'

$script:CmdLog = New-Object 'System.Collections.Generic.List[string]'
$script:StartedPids = New-Object 'System.Collections.Generic.List[int]'
$script:OldExePath = $null
$script:NewExePath = $null
$script:DataBackup = $null

# 异常中断时也要收尾：只结束本脚本启动的进程、恢复数据目录。
function Stop-AllStartedApps {
    foreach ($procId in @($script:StartedPids)) {
        $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
        if (-not $p) { continue }
        $path = $null
        try { $path = $p.Path } catch { $path = $null }
        if (-not $path) { continue }
        $isTarget = $false
        foreach ($candidate in @($script:OldExePath, $script:NewExePath)) {
            if (-not $candidate) { continue }
            try {
                if ([string]::Equals(
                        [System.IO.Path]::GetFullPath($path),
                        [System.IO.Path]::GetFullPath($candidate),
                        [StringComparison]::OrdinalIgnoreCase)) { $isTarget = $true }
            } catch { }
        }
        if ($isTarget) {
            Write-Host ("  ! 收尾：结束本脚本启动的进程 PID " + $procId) -ForegroundColor Yellow
            Stop-TestApp $procId $path
        }
    }
    $script:StartedPids.Clear()
}

function Restore-DataDir {
    $backup = $script:DataBackup
    if (-not $backup) { return }
    if (-not (Test-Path -LiteralPath $backup -PathType Container)) {
        $script:DataBackup = $null
        return
    }
    $appsRunning = @()
    foreach ($candidate in @($script:OldExePath, $script:NewExePath)) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf) -and
            (@(Find-RelatedProcesses $candidate).Count -gt 0)) {
            $appsRunning += $candidate
        }
    }
    if ($appsRunning.Count -gt 0) {
        Write-Host '! 仍有应用在运行，跳过数据目录恢复（避免覆盖用户数据）' -ForegroundColor Yellow
    } else {
        Write-Cmd ("Remove-Item -LiteralPath '" + $DataDir + "' -Recurse -Force; Copy-Item -LiteralPath '" +
                   $backup + "' -Destination '" + $DataDir + "' -Recurse -Force")
        if (Test-Path -LiteralPath $DataDir) { Remove-Item -LiteralPath $DataDir -Recurse -Force }
        Copy-Item -LiteralPath $backup -Destination $DataDir -Recurse -Force
        Write-Host '  数据目录已恢复到测量前状态' -ForegroundColor Green
    }
    if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue }
    $script:DataBackup = $null
}

trap {
    Write-Host ("! 脚本异常中断：" + $_.Exception.Message) -ForegroundColor Red
    try { Stop-AllStartedApps } catch { }
    try { Restore-DataDir } catch { }
    exit 1
}

# ---------------------------------------------------------------- 输出小工具

function Write-Cmd([string]$CommandLine) {
    $script:CmdLog.Add($CommandLine)
    Write-Host ("  > " + $CommandLine) -ForegroundColor DarkCyan
}

function Write-Step([string]$Message) {
    Write-Host ("▸ " + $Message) -ForegroundColor Cyan
}

function Format-MB([object]$Bytes) {
    if ($null -eq $Bytes) { return '—' }
    $b = [double]$Bytes
    if ($b -le 0) { return '0 MB' }
    if ($b -lt 1MB) { return ('{0:N1} KB' -f ($b / 1KB)) }
    return ('{0:N2} MB' -f ($b / 1MB))
}

# ---------------------------------------------------------------- 体积测量

function Get-FileSizeOrNull([string]$Path) {
    if ($Path -and (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return [int64](Get-Item -LiteralPath $Path).Length
    }
    return $null
}

function Get-DirLogicalSize([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return $null }
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue |
            Measure-Object -Property Length -Sum).Sum
    if ($null -eq $sum) { return [int64]0 }
    return [int64]$sum
}

function Get-DirAllocatedSize([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return $null }
    $total = [int64]0
    Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue | ForEach-Object {
        $total += [int64]([math]::Ceiling($_.Length / $ClusterSize) * $ClusterSize)
    }
    return $total
}

function Get-TopLevelFilesSize([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return $null }
    $sum = (Get-ChildItem -LiteralPath $Path -File -Force -ErrorAction SilentlyContinue |
            Measure-Object -Property Length -Sum).Sum
    if ($null -eq $sum) { return [int64]0 }
    return [int64]$sum
}

function Resolve-FirstExisting([string[]]$Candidates) {
    foreach ($c in $Candidates) {
        if ($c -and (Test-Path -LiteralPath $c -PathType Leaf)) { return $c }
    }
    return $null
}

function Resolve-LatestFile([string]$Dir, [string]$Filter) {
    if (-not (Test-Path -LiteralPath $Dir -PathType Container)) { return $null }
    $f = Get-ChildItem -LiteralPath $Dir -Filter $Filter -File -ErrorAction SilentlyContinue |
         Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if ($f) { return $f.FullName }
    return $null
}

# ---------------------------------------------------------------- 安装信息（只读）

function Get-UninstallEntries([string]$DisplayName) {
    $paths = @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )
    Get-ItemProperty -Path $paths -ErrorAction SilentlyContinue |
        Where-Object { $_.DisplayName -eq $DisplayName }
}

function Get-InstallDirFromEntry($Entry) {
    if ($null -eq $Entry) { return $null }
    $loc = $Entry.InstallLocation
    if ($loc) {
        $loc = ([string]$loc).Trim().Trim('"')
        if (Test-Path -LiteralPath $loc) { return $loc }
    }
    $un = $Entry.UninstallString
    if ($un) {
        $u = ([string]$un).Trim()
        if ($u.StartsWith('"')) {
            $end = $u.IndexOf('"', 1)
            if ($end -gt 0) { return Split-Path -Parent $u.Substring(1, $end - 1) }
        }
        $first = ($u -split '\s+')[0]
        if (Test-Path -LiteralPath $first) { return Split-Path -Parent $first }
    }
    return $null
}

function Test-ElectronInstall([string]$Dir) {
    return (Test-Path -LiteralPath (Join-Path $Dir 'resources\app.asar'))
}

# 用 7-Zip / Bandizip 解包安装器，估算安装后占用（不改动系统状态）。
function Expand-InstallerForSize([string]$Installer) {
    $tool = $null
    $kind = $null
    foreach ($c in @('7z', '7za', '7zr')) {
        $cmd = Get-Command $c -ErrorAction SilentlyContinue
        if ($cmd) { $tool = $cmd.Source; $kind = '7z'; break }
    }
    if (-not $tool) {
        $bz = 'C:\Program Files\Bandizip\bz.exe'
        if (Test-Path -LiteralPath $bz) { $tool = $bz; $kind = 'bz' }
    }
    if (-not $tool) { return $null }

    $work = Join-Path $env:TEMP ('sms-measure-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    try {
        $outer = Join-Path $work 'outer'
        New-Item -ItemType Directory -Path $outer -Force | Out-Null
        if ($kind -eq '7z') {
            Write-Cmd ("& '{0}' x -y -o`"{1}`" `"{2}`"" -f $tool, $outer, $Installer)
            & $tool x -y ("-o" + $outer) $Installer | Out-Null
        } else {
            Write-Cmd ("& '{0}' x -y -o:`"{1}`" `"{2}`"" -f $tool, $outer, $Installer)
            & $tool x -y ("-o:" + $outer) $Installer | Out-Null
        }

        # Tauri NSIS: 文件直接以 $INSTDIR\... 形式存放。
        $instDir = Join-Path $outer '$INSTDIR'
        if (Test-Path -LiteralPath $instDir -PathType Container) {
            return (Get-DirAllocatedSize $instDir)
        }

        # Electron-builder NSIS: 内层 app-64.7z 才是应用本体。
        $payload = Join-Path $outer '$PLUGINSDIR\app-64.7z'
        if (-not (Test-Path -LiteralPath $payload)) {
            $payload = Get-ChildItem -LiteralPath (Join-Path $outer '$PLUGINSDIR') -Filter 'app-64.7z' -File -ErrorAction SilentlyContinue |
                       Select-Object -First 1 -ExpandProperty FullName
        }
        if ($payload -and (Test-Path -LiteralPath $payload)) {
            $inner = Join-Path $work 'inner'
            New-Item -ItemType Directory -Path $inner -Force | Out-Null
            if ($kind -eq '7z') {
                Write-Cmd ("& '{0}' x -y -o`"{1}`" `"{2}`"" -f $tool, $inner, $payload)
                & $tool x -y ("-o" + $inner) $payload | Out-Null
            } else {
                Write-Cmd ("& '{0}' x -y -o:`"{1}`" `"{2}`"" -f $tool, $inner, $payload)
                & $tool x -y ("-o:" + $inner) $payload | Out-Null
            }
            return (Get-DirAllocatedSize $inner)
        }

        # 兜底：外层除 $PLUGINSDIR / $R0 之外的内容。
        $total = [int64]0
        Get-ChildItem -LiteralPath $outer -Recurse -File -Force -ErrorAction SilentlyContinue | ForEach-Object {
            $full = $_.FullName.Substring($outer.Length).TrimStart('\')
            if ($full -like '$PLUGINSDIR*' -or $full -like '$R0*') { return }
            $total += [int64]([math]::Ceiling($_.Length / $ClusterSize) * $ClusterSize)
        }
        return $total
    } catch {
        Write-Host ("  ! 解包失败：" + $_.Exception.Message) -ForegroundColor Yellow
        return $null
    } finally {
        # 只删自己建的临时目录。
        if ((Test-Path -LiteralPath $work) -and $work -like (Join-Path $env:TEMP 'sms-measure-*')) {
            Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# ---------------------------------------------------------------- 进程工具

function Find-ExeProcesses([string]$ExePath) {
    if (-not $ExePath) { return @() }
    $full = [System.IO.Path]::GetFullPath($ExePath)
    $result = @()
    foreach ($p in @(Get-Process -ErrorAction SilentlyContinue)) {
        $path = $null
        try { $path = $p.Path } catch { $path = $null }
        if (-not $path) { continue }
        try {
            if ([string]::Equals([System.IO.Path]::GetFullPath($path), $full, [StringComparison]::OrdinalIgnoreCase)) {
                $result += $p
            }
        } catch { }
    }
    return $result
}

# 同名进程（旧 Electron 与 Tauri 主程序都叫 "Windows SMS Bridge"，无法只靠名字区分）。
function Find-RelatedProcesses([string]$ExePath) {
    if (-not $ExePath) { return @() }
    $name = [System.IO.Path]::GetFileNameWithoutExtension($ExePath)
    if (-not $name) { return @() }
    return @(Get-Process -Name $name -ErrorAction SilentlyContinue)
}

function Start-TestApp([string]$ExePath) {
    Write-Cmd ("Start-Process -FilePath '" + $ExePath + "' -PassThru")
    $proc = Start-Process -FilePath $ExePath -PassThru
    if ($null -eq $proc) { return $null }
    $script:StartedPids.Add($proc.Id)
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline) {
        $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
        if (-not $p) { return $null }   # 启动即退出（例如已有实例）
        try { $p.Refresh() } catch { }
        if ($p.MainWindowHandle -ne 0) { return $p }
        Start-Sleep -Milliseconds 500
    }
    return (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)
}

function Stop-TestApp([int]$ProcessId, [string]$ExePath) {
    $p = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if (-not $p) { return }
    $path = $null
    try { $path = $p.Path } catch { $path = $null }
    $sameExe = $false
    if ($path) {
        try {
            $sameExe = [string]::Equals(
                [System.IO.Path]::GetFullPath($path),
                [System.IO.Path]::GetFullPath($ExePath),
                [StringComparison]::OrdinalIgnoreCase)
        } catch { $sameExe = $false }
    }
    if (-not $sameExe) {
        Write-Host ("  ! PID {0} 已不是目标可执行文件，跳过结束" -f $ProcessId) -ForegroundColor Yellow
        return
    }
    # 只结束自己启动的进程树。
    Write-Cmd ("taskkill /PID {0} /T /F" -f $ProcessId)
    & taskkill.exe /PID $ProcessId /T /F 2>&1 | Out-Null
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        if (-not (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) { break }
        Start-Sleep -Milliseconds 300
    }
}

function Get-ProcessMemory([int]$ProcessId) {
    $p = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if (-not $p) { return $null }
    return [pscustomobject]@{
        WS  = [int64]$p.WorkingSet64
        PWS = [int64]$p.PrivateMemorySize64
    }
}

function Measure-MemoryWindow([int]$ProcessId, [int]$Seconds) {
    $pwsPeak = [int64]0
    $wsPeak = [int64]0
    $last = $null
    $deadline = (Get-Date).AddSeconds([math]::Max(1, $Seconds))
    while ((Get-Date) -lt $deadline) {
        $m = Get-ProcessMemory $ProcessId
        if ($null -eq $m) { break }
        if ($m.PWS -gt $pwsPeak) { $pwsPeak = $m.PWS }
        if ($m.WS -gt $wsPeak) { $wsPeak = $m.WS }
        $last = $m
        Start-Sleep -Milliseconds 1000
    }
    if ($null -eq $last) { return $null }
    return [pscustomobject]@{
        PwsPeak   = $pwsPeak
        PwsSteady = $last.PWS
        WsPeak    = $wsPeak
        WsSteady  = $last.WS
    }
}

# ---------------------------------------------------------------- 同步检测

function Get-DataStamp {
    if (-not (Test-Path -LiteralPath $DataDir -PathType Container)) { return '' }
    $parts = @()
    foreach ($name in @('config.json', 'messages.json')) {
        $f = Join-Path $DataDir $name
        if (Test-Path -LiteralPath $f -PathType Leaf) {
            $item = Get-Item -LiteralPath $f
            $parts += ('{0}:{1}:{2}' -f $name, $item.LastWriteTimeUtc.Ticks, $item.Length)
        }
    }
    return ($parts -join '|')
}

function Wait-DataChange([string]$Before, [int]$TimeoutSec) {
    if ($TimeoutSec -le 0) { return $false }
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        if ((Get-DataStamp) -ne $Before) { return $true }
        Start-Sleep -Seconds 2
    }
    return $false
}

# ---------------------------------------------------------------- 单应用内存测量

function Measure-AppMemory([string]$Label, [string]$ExePath, [bool]$AllowRelatedReuse = $false) {
    # List 而不是数组：$info.Notes 持有同一个引用，追加的说明才能带出去。
    $notes = New-Object 'System.Collections.Generic.List[string]'
    $idle = $null
    $sync = $null
    $info = [pscustomobject]@{
        Label    = $Label
        Idle     = $null
        Sync     = $null
        Skipped  = $null
        Notes    = $notes
    }

    if (-not $ExePath -or -not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
        $info.Skipped = '可执行文件不存在：' + $(if ($ExePath) { $ExePath } else { '(未找到)' })
        Write-Host ("  ! " + $info.Skipped) -ForegroundColor Yellow
        return $info
    }

    $running = @(Find-ExeProcesses $ExePath)
    $launchedPid = $null
    $mainPid = $null
    $reused = $false

    if ($running.Count -gt 0) {
        $reused = $true
        $main = $running | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if (-not $main) { $main = $running[0] }
        $mainPid = $main.Id
        $notes.Add('已存在运行实例，复用只读采样（PID ' + $mainPid + '），未结束它')
        Write-Host ("  ! " + $Label + " 已在运行（PID " + $mainPid + "），复用现有实例采样") -ForegroundColor Yellow
    } else {
        $proc = Start-TestApp $ExePath
        if ($null -eq $proc) {
            # 指定路径的实例启动后立即退出：常见原因是同类实例已在运行、单实例锁生效。
            $related = @(Find-RelatedProcesses $ExePath)
            if ($related.Count -gt 0 -and $AllowRelatedReuse) {
                $reused = $true
                $main = $related | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
                if (-not $main) { $main = $related[0] }
                $mainPid = $main.Id
                $relatedPath = $null
                try { $relatedPath = $main.Path } catch { $relatedPath = $null }
                if (-not $relatedPath) { $relatedPath = '(未知路径)' }
                $notes.Add('指定路径的实例未能独立启动（可能已有同类实例在运行），改为只读采样已有实例 PID ' +
                           $mainPid + '：' + $relatedPath)
                Write-Host ("  ! " + $Label + " 启动后立即退出，改为只读采样已有实例 PID " + $mainPid) -ForegroundColor Yellow
            } else {
                if ($related.Count -gt 0) {
                    $notes.Add('启动后立即退出，且只找到同名进程（无法确认是指定实例），跳过以避免测错')
                    $info.Skipped = '启动失败（可能已有同类实例）；同名进程无法确认，跳过'
                } else {
                    $info.Skipped = '启动失败或启动后立即退出'
                }
                Write-Host ("  ! " + $info.Skipped) -ForegroundColor Yellow
                return $info
            }
        } else {
            $mainPid = $proc.Id
            $launchedPid = $mainPid
            Write-Step ($Label + " 已启动（PID " + $mainPid + "），窗口空闲 " + $IdleSeconds + " s …")
            Start-Sleep -Seconds $IdleSeconds
        }
    }

    $idle = Measure-MemoryWindow $mainPid $SampleSeconds
    if ($null -eq $idle) {
        $info.Skipped = '采样期间进程已退出'
        return $info
    }

    # ---- 同步后
    if ($SyncCommand) {
        $before = Get-DataStamp
        Write-Cmd $SyncCommand
        try { Invoke-Expression $SyncCommand | Out-Null } catch {
            $notes.Add('SyncCommand 执行失败：' + $_.Exception.Message)
            Write-Host ("  ! SyncCommand 执行失败：" + $_.Exception.Message) -ForegroundColor Yellow
        }
        $changed = Wait-DataChange $before $SyncTimeoutSeconds
        if (-not $changed) { $notes.Add('等待 ' + $SyncTimeoutSeconds + ' s 未观察到数据文件变化，仍按同步后采样') }
        Start-Sleep -Seconds 3
        $sync = Measure-MemoryWindow $mainPid $SampleSeconds
        if ($changed) {
            $info.Notes.Add('同步方式：外部命令（检测到数据写入）')
        } else {
            $info.Notes.Add('同步方式：外部命令（未检测到数据写入）')
        }
    } elseif ($launchedPid -and -not $reused) {
        # 重启触发 syncOnStart（两个客户端都有该行为），等数据文件变化后采样。
        Write-Step ($Label + " 重启以触发启动同步 …")
        $before = Get-DataStamp
        Stop-TestApp $launchedPid $ExePath
        $script:StartedPids.Remove($launchedPid) | Out-Null
        $proc = Start-TestApp $ExePath
        if ($null -eq $proc) {
            $notes.Add('重启失败，同步后数据缺失')
        } else {
            $launchedPid = $proc.Id
            $changed = Wait-DataChange $before $SyncTimeoutSeconds
            if (-not $changed) { $notes.Add('等待 ' + $SyncTimeoutSeconds + ' s 未观察到数据文件变化（可能未配对手机），仍按同步后采样') }
            Start-Sleep -Seconds 3
            $sync = Measure-MemoryWindow $proc.Id $SampleSeconds
            if ($changed) {
                $info.Notes.Add('同步方式：重启触发启动同步（检测到数据写入）')
            } else {
                $info.Notes.Add('同步方式：重启触发启动同步（未检测到数据写入）')
            }
        }
    } else {
        $notes.Add('应用已在运行且未提供 -SyncCommand，无法在不结束它的前提下触发同步，同步后数据缺失')
    }

    # ---- 收尾：只结束自己启动的进程
    if ($launchedPid -and (Get-Process -Id $launchedPid -ErrorAction SilentlyContinue)) {
        Write-Step ($Label + " 测量结束，结束本脚本启动的进程 PID " + $launchedPid)
        Stop-TestApp $launchedPid $ExePath
    }
    $script:StartedPids.Remove($launchedPid) | Out-Null

    $info.Idle = $idle
    $info.Sync = $sync
    return $info
}

# ================================================================ 主流程

$notes = New-Object 'System.Collections.Generic.List[string]'
$rows = New-Object 'System.Collections.Generic.List[object]'

function Add-Row([string]$Metric, [object]$Old, [object]$New) {
    $rows.Add([pscustomobject]@{ Metric = $Metric; Old = $Old; New = $New }) | Out-Null
}

Write-Step "路径"
Write-Host ("  旧 Electron: " + $OldRoot)
Write-Host ("  新 Tauri   : " + $NewRoot)
Write-Host ("  数据目录   : " + $DataDir)

# ---- 定位文件
$oldUnpacked = Join-Path $OldRoot 'dist\win-unpacked'
$oldExe = Resolve-FirstExisting @(
    (Join-Path $oldUnpacked 'Windows SMS Bridge.exe'),
    (Join-Path $oldUnpacked 'windows-sms-bridge.exe')
)
$oldInstaller = Resolve-LatestFile (Join-Path $OldRoot 'dist') 'Windows-SMS-Bridge-Setup-*.exe'

$newRelease = Join-Path $NewRoot 'target\release'
$newExe = Resolve-FirstExisting @(
    (Join-Path $newRelease 'Windows SMS Bridge.exe'),
    (Join-Path $newRelease 'windows-sms-bridge.exe')
)
$newInstaller = Resolve-LatestFile (Join-Path $newRelease 'bundle\nsis') '*.exe'

$oldExeDesc = if ($oldExe) { $oldExe } else { '(缺失)' }
$newExeDesc = if ($newExe) { $newExe } else { '(缺失，先 cargo build --release)' }
$script:OldExePath = $oldExe
$script:NewExePath = $newExe
Write-Host ("  旧程序     : " + $oldExeDesc)
Write-Host ("  新程序     : " + $newExeDesc)
Write-Host ("  旧安装包   : " + $(if ($oldInstaller) { $oldInstaller } else { '(缺失)' }))
Write-Host ("  新安装包   : " + $(if ($newInstaller) { $newInstaller } else { '(缺失，先 npm run dist)' }))

# ---- 1) 安装包大小
Write-Step '安装包大小'
Add-Row '安装包大小' (Format-MB (Get-FileSizeOrNull $oldInstaller)) (Format-MB (Get-FileSizeOrNull $newInstaller))

# ---- 2) 解包目录大小
Write-Step '解包目录大小'
$oldUnpackedSize = $null
if (Test-Path -LiteralPath $oldUnpacked) {
    $oldUnpackedSize = Get-DirLogicalSize $oldUnpacked
    $notes.Add('旧 Electron 解包大小 = dist\win-unpacked 全部文件逻辑大小')
} else {
    $notes.Add('旧 Electron dist\win-unpacked 不存在，解包大小缺失')
}
$newUnpackedSize = $null
if ($newExe) {
    $newUnpackedSize = Get-TopLevelFilesSize $newRelease
    $notes.Add('新 Tauri 解包大小 = target\release 顶层文件（Tauri 单 exe，不含 deps/build 等中间产物）')
} else {
    $notes.Add('新 Tauri target\release 可执行文件不存在，解包大小缺失')
}
Add-Row '解包目录大小' (Format-MB $oldUnpackedSize) (Format-MB $newUnpackedSize)

# ---- 3) 安装后占用
Write-Step '安装后占用'
$oldInstalled = $null
$newInstalled = $null
if ($SkipInstallSize) {
    $notes.Add('已用 -SkipInstallSize 跳过安装后占用')
} else {
    $entries = @(Get-UninstallEntries $ProductName)
    $oldEntry = $null
    $newEntry = $null
    foreach ($e in $entries) {
        $dir = Get-InstallDirFromEntry $e
        if (-not $dir) { continue }
        if (Test-ElectronInstall $dir) {
            if (-not $oldEntry) { $oldEntry = $dir }
        } elseif (-not $newEntry) {
            $newEntry = $dir
        }
    }
    if ($oldEntry) {
        $oldInstalled = Get-DirAllocatedSize $oldEntry
        $notes.Add('旧 Electron 安装后占用 = 已安装目录（' + $oldEntry + '，4 KiB 簇分配）')
    } elseif ($oldInstaller) {
        $oldInstalled = Expand-InstallerForSize $oldInstaller
        if ($null -ne $oldInstalled) { $notes.Add('旧 Electron 未安装，安装后占用 = 安装器解包估算（4 KiB 簇分配）') }
    }
    if ($newEntry) {
        $newInstalled = Get-DirAllocatedSize $newEntry
        $notes.Add('新 Tauri 安装后占用 = 已安装目录（' + $newEntry + '，4 KiB 簇分配）')
    } elseif ($newInstaller) {
        $newInstalled = Expand-InstallerForSize $newInstaller
        if ($null -ne $newInstalled) { $notes.Add('新 Tauri 未安装，安装后占用 = 安装器解包估算（4 KiB 簇分配）') }
    }
    if ($null -eq $oldInstalled) { $notes.Add('旧 Electron 安装后占用缺失（未安装且无安装器/解包工具）') }
    if ($null -eq $newInstalled) { $notes.Add('新 Tauri 安装后占用缺失（未安装且无安装器/解包工具）') }
}
Add-Row '安装后占用（磁盘）' (Format-MB $oldInstalled) (Format-MB $newInstalled)

# ---- 4) 内存
$oldMem = $null
$newMem = $null
$preexisting = @()
foreach ($pair in @(
        @{ Name = '旧 Electron'; Exe = $oldExe },
        @{ Name = '新 Tauri'; Exe = $newExe })) {
    if ($pair.Exe -and (Test-Path -LiteralPath $pair.Exe -PathType Leaf)) {
        # 旧 Electron 与 Tauri 主程序同名，无法靠进程名区分版本，只能保守判断"应用正在运行"。
        if (@(Find-RelatedProcesses $pair.Exe).Count -gt 0) { $preexisting += $pair.Name }
    }
}
$cleanRun = ($preexisting.Count -eq 0)
if (-not $cleanRun) {
    Write-Host ("! 检测到 Windows SMS Bridge 进程正在运行（旧/新版本同名，无法区分）。脚本不会结束它们；已运行的一方只做只读采样。") -ForegroundColor Yellow
    if ($RequireStopped) { throw "-RequireStopped：请先关闭所有 Windows SMS Bridge 进程再运行。" }
}

$script:DataBackup = $null
if (-not $SkipMemory) {
    # 干净运行时备份数据目录，测量结束后恢复（"restore state"）。
    if ($cleanRun -and -not $SkipDataBackup -and (Test-Path -LiteralPath $DataDir -PathType Container)) {
        $script:DataBackup = Join-Path $env:TEMP ('sms-bridge-data-backup-' + [guid]::NewGuid().ToString('N'))
        Write-Cmd ("Copy-Item -LiteralPath '" + $DataDir + "' -Destination '" + $script:DataBackup + "' -Recurse -Force")
        Copy-Item -LiteralPath $DataDir -Destination $script:DataBackup -Recurse -Force
        $notes.Add('已备份 %APPDATA%\' + $ProductName + '，测量结束后恢复')
    } elseif (-not $cleanRun) {
        $notes.Add('有应用实例原本就在运行，未备份/恢复数据目录（避免覆盖用户数据）')
    } elseif ($SkipDataBackup) {
        $notes.Add('已用 -SkipDataBackup 跳过数据目录备份/恢复')
    }

    Write-Step '旧 Electron 内存测量'
    $oldMem = Measure-AppMemory '旧 Electron' $oldExe $true
    foreach ($n in $oldMem.Notes) { $notes.Add('旧 Electron：' + $n) }

    Write-Step '新 Tauri 内存测量'
    $newMem = Measure-AppMemory '新 Tauri' $newExe $false
    foreach ($n in $newMem.Notes) { $notes.Add('新 Tauri：' + $n) }
} else {
    $notes.Add('已用 -SkipMemory 跳过内存测量')
}

function Get-MemValue($Mem, [string]$Field) {
    if ($null -eq $Mem -or $null -eq $Mem.Idle) { return $null }
    return $Mem.Idle.$Field
}
function Get-MemValueSync($Mem, [string]$Field) {
    if ($null -eq $Mem -or $null -eq $Mem.Sync) { return $null }
    return $Mem.Sync.$Field
}

$idleLabel = '空闲 ' + $IdleSeconds + 's 后 '
Add-Row ($idleLabel + '主进程 Private WS 峰值') (Format-MB (Get-MemValue $oldMem 'PwsPeak')) (Format-MB (Get-MemValue $newMem 'PwsPeak'))
Add-Row ($idleLabel + '主进程 Private WS 稳定') (Format-MB (Get-MemValue $oldMem 'PwsSteady')) (Format-MB (Get-MemValue $newMem 'PwsSteady'))
Add-Row ($idleLabel + '主进程 Working Set 稳定') (Format-MB (Get-MemValue $oldMem 'WsSteady')) (Format-MB (Get-MemValue $newMem 'WsSteady'))
Add-Row '同步后 主进程 Private WS 峰值' (Format-MB (Get-MemValueSync $oldMem 'PwsPeak')) (Format-MB (Get-MemValueSync $newMem 'PwsPeak'))
Add-Row '同步后 主进程 Private WS 稳定' (Format-MB (Get-MemValueSync $oldMem 'PwsSteady')) (Format-MB (Get-MemValueSync $newMem 'PwsSteady'))
Add-Row '同步后 主进程 Working Set 稳定' (Format-MB (Get-MemValueSync $oldMem 'WsSteady')) (Format-MB (Get-MemValueSync $newMem 'WsSteady'))

if ($oldMem -and $oldMem.Skipped) { $notes.Add('旧 Electron 内存：' + $oldMem.Skipped) }
if ($newMem -and $newMem.Skipped) { $notes.Add('新 Tauri 内存：' + $newMem.Skipped) }
$notes.Add('PWS = PrivateMemorySize64（Private Working Set），WS = WorkingSet64；每 1 s 采样，峰值 = 采样窗口内最大值，稳定 = 最后一次采样')
$notes.Add('测量顺序：先旧 Electron，后新 Tauri；每次只结束本脚本启动的进程树')

# ---- 收尾：结束残留的测量进程，再恢复数据目录
Stop-AllStartedApps
if ($script:DataBackup) { Write-Step '恢复数据目录' }
Restore-DataDir

# ---- 输出 markdown
$md = New-Object 'System.Collections.Generic.List[string]'
$md.Add('# 体积 / 内存测量：Windows SMS Bridge') | Out-Null
$md.Add('') | Out-Null
$md.Add('- 生成时间：' + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')) | Out-Null
$md.Add('- 旧 Electron 程序：' + $oldExeDesc) | Out-Null
$md.Add('- 新 Tauri 程序：' + $newExeDesc) | Out-Null
$md.Add('- 旧安装包：' + $(if ($oldInstaller) { $oldInstaller } else { '缺失' })) | Out-Null
$md.Add('- 新安装包：' + $(if ($newInstaller) { $newInstaller } else { '缺失' })) | Out-Null
$md.Add('') | Out-Null
$md.Add('| 指标 | 旧 Electron | 新 Tauri |') | Out-Null
$md.Add('| --- | --- | --- |') | Out-Null
foreach ($r in $rows) {
    $md.Add('| ' + $r.Metric + ' | ' + $r.Old + ' | ' + $r.New + ' |') | Out-Null
}
$md.Add('') | Out-Null
$md.Add('## 说明') | Out-Null
$md.Add('') | Out-Null
foreach ($n in $notes) { $md.Add('- ' + $n) | Out-Null }
$md.Add('') | Out-Null
$md.Add('## 本次执行的命令') | Out-Null
$md.Add('') | Out-Null
$md.Add('```powershell') | Out-Null
foreach ($c in $script:CmdLog) { $md.Add($c) | Out-Null }
$md.Add('```') | Out-Null

$text = ($md -join "`n")

if (-not (Test-Path -LiteralPath $OutDir -PathType Container)) {
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}
Write-Cmd ("Set-Content -LiteralPath '" + $OutFile + "' -Encoding UTF8")
Set-Content -LiteralPath $OutFile -Value $text -Encoding UTF8

Write-Host ''
Write-Output $text

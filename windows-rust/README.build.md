# Windows 客户端构建说明（Tauri 2）

`windows-rust\` 是用 Tauri 2 重写的 Windows 客户端，替换旧的 Electron 客户端（`..\windows\`，只读参考）。

## 环境

- Rust stable（>= 1.77，MSVC toolchain）
- Node.js + npm（只用来跑 Tauri CLI / NSIS 打包）
- WebView2 Runtime：使用系统已有版本（配置里 `webviewInstallMode: {"type":"skip"}`，不打包、不下载）

## 构建

```powershell
cd H:\Project\hermes-android-sms\windows-rust

cargo build --release        # 只编译二进制
npm install                  # 一次性：安装 @tauri-apps/cli（自带 NSIS 打包器）
npm run dist                 # = tauri build：编译 + 打 NSIS 安装器
```

`npm run build` = `tauri build --no-bundle`（只要二进制，不打安装器）。

产物位置：

| 产物 | 路径 |
| --- | --- |
| 可执行文件 | `target\release\Windows SMS Bridge.exe`（`mainBinaryName` 决定文件名） |
| NSIS 安装器 | `target\release\bundle\nsis\Windows SMS Bridge_2.1.0_x64-setup.exe` |
| 打包中间文件 | `target\release\bundle\nsis\`（`installer.nsi`、图标、语言文件等） |

## 安装器命名（重要）

更新器与 GitHub Release 约定安装包名为 `Windows-SMS-Bridge-Setup-<version>.exe`
（`updater.rs` 里对应 `^Windows-SMS-Bridge-Setup-.*\.exe$`）。

Tauri 2 的 NSIS 打包器把输出文件名固定为 `<productName>_<version>_<arch>-setup.exe`；
`tauri.conf.json` **没有** `artifactName` / `uninstallDisplayName` 之类的键
（已核对本机 `tauri-utils-2.10.2` 的 `BundleConfig` / `NsisConfig`，配置结构是 `deny_unknown_fields`）。
因此上传 Release 前需要改名：

```powershell
$ver = (Get-Content tauri.conf.json | ConvertFrom-Json).version
$src = Get-ChildItem target\release\bundle\nsis\*_x64-setup.exe | Select-Object -First 1
Rename-Item $src.FullName ("Windows-SMS-Bridge-Setup-$ver.exe")
```

## 配置项（`tauri.conf.json` → `bundle`）

| 键 | 值 | 作用 |
| --- | --- | --- |
| `targets` | `["nsis"]` | 只产出 NSIS 安装器 |
| `windows.nsis.installMode` | `currentUser` | 装到 HKCU；NSIS 模板用 `RequestExecutionLevel user`，不弹 UAC |
| `windows.nsis.displayLanguageSelector` | `false` | 按系统语言自动选 SimpChinese / English / Japanese |
| `windows.nsis.installerIcon` / `uninstallerIcon` | `assets/icon.ico` | 安装器 / 卸载器图标 |
| `windows.nsis.compression` | `lzma` | 安装器压缩算法 |
| `windows.webviewInstallMode` | `{"type":"skip"}` | 不打包 WebView2，直接用系统运行时 |
| `icon` | `assets/icon.ico`（另有 png 供开发期图标） | Windows 资源用 `.ico` |
| `mainBinaryName` | `Windows SMS Bridge` | 安装后的主程序名：`%LOCALAPPDATA%\Windows SMS Bridge\Windows SMS Bridge.exe` |
| `productName` | `Windows SMS Bridge` | 决定桌面/开始菜单快捷方式名、卸载列表显示名、安装目录名 |

NSIS 模板（Tauri 2）的行为：`installMode=currentUser` 时安装到
`%LOCALAPPDATA%\Windows SMS Bridge\`，自动创建
`桌面\Windows SMS Bridge.lnk` 与 `开始菜单\Windows SMS Bridge.lnk`，
卸载列表 `DisplayName` = `productName`，安装的主程序 = `$INSTDIR\${MAINBINARYNAME}.exe`。

## 体积 / 内存测量

`scripts\measure.ps1`（查看完整参数：`Get-Help .\scripts\measure.ps1 -Full`）：

```powershell
pwsh -File scripts\measure.ps1
```

按顺序测量旧 Electron（`..\windows\dist\win-unpacked`）与新 Tauri（`target\release`）：
安装包大小、解包目录大小、安装后磁盘占用、空闲 60 s 与同步后的主进程
Private Working Set / Working Set；结果同时打印到控制台和 `.shots\measure.md`。
只结束脚本自己启动的进程树，已在运行的应用只做只读采样。

## 其他脚本

- `npm run i18n:gen` — 生成 `src\i18n_table.rs`
- `npm run i18n:check` — 前端 i18n 扫描
- `npm run shim:check` — 校验 `web\` 的 `window.sms` shim 覆盖（沿用旧脚本）

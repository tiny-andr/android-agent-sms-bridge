# Windows 客户端构建说明（Tauri 2）

`windows-rust\` 是用 Tauri 2 重写的 Windows 客户端。旧的 Electron 客户端已从工作区移除，
源码仍保留在 git 历史（提交 `cd17dda`）；需要对照时：`git checkout cd17dda -- windows`。

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

按顺序测量旧 Electron 与新 Tauri（`target\release`）。旧客户端目录不在工作区时，旧的一侧显示“—”；
要补全就从 git 历史恢复（`git checkout cd17dda -- windows`）并 `npm ci && npm run pack`：
安装包大小、解包目录大小、安装后磁盘占用、空闲 60 s 与同步后的主进程
Private Working Set / Working Set；结果同时打印到控制台和 `.shots\measure.md`。
只结束脚本自己启动的进程树，已在运行的应用只做只读采样。

## 验收 / 自查（每一步都可复跑）

```powershell
cd H:\Project\hermes-android-sms\windows-rust

cargo test                                   # 37 个单元测试
cargo test --test live_protocol -- --ignored # 真机：签名请求必须 200，错证书必须被拒
cargo test --test live_updater  -- --ignored # 真 GitHub release：读版本 + digest
$env:RUN_UPDATE_DOWNLOAD='1'; cargo test --test live_updater live_update_download -- --ignored --nocapture  # 真下载 + SHA-256 校验
node scripts\shim:check 2>$null; node scripts\check-shim.cjs   # 22 通道契约
npm run i18n:check                            # 三语扫描（需 NODE_PATH 指向 ..\windows\node_modules）
powershell -File scripts\parity-sync.ps1      # 新旧客户端同机同步，逐字段对比缓存
pwsh -File scripts\measure.ps1                # 体积 / 内存对比
```

说明：

- `i18n:check` 需要本机有 Electron（只跑这个扫描时临时装即可：`npm install --no-save electron`），
  并把 `$env:NODE_PATH` 指向 `windows-rust\node_modules`。
  （它借 Electron 渲染真实页面），并先 `Remove-Item Env:\ELECTRON_RUN_AS_NODE`。
  改完后跑出来是 28 个状态 / 13 个失败 / 40 处缺陷，**与旧 Electron 渲染器的报告逐条完全相同**
  （这些是旧版 app.js 里的历史遗留，不是重写引入的）。
- `parity-sync.ps1` 会各给两个客户端一份干净 profile（保留配对、清空缓存），
  双方各跑一次完整 30 天同步，然后按 id 对比每一条短信的
  address/body/date/type/slot/subId/carrier/simNumber/read 以及会话聚合结果。
- 真机与网络相关的测试用 `--ignored` 标注，CI 里默认跳过。

## 实测数据（2026-11-10，本机）

| 指标 | 旧 Electron | 新 Tauri |
| --- | --- | --- |
| 安装包 | 83.69 MB | **2.22 MB** |
| 解包目录 | 268.56 MB | 单 exe **5.37 MB** |
| 安装后磁盘占用 | 268.91 MB | **5.60 MB** |
| 空闲 60 s 主进程 Private WS | 79.84 MB | **14.14 MB** |
| 空闲 60 s 主进程 Working Set | 116.32 MB | **41.90 MB** |
| 空闲 60 s 全进程树 Private WS | 384.69 MB | **281.22 MB**（7 个进程：主进程 + 6 个 WebView2） |
| 全进程树 Working Set | 514.86 MB | **492.21 MB** |

体积差主要来自“不再打包 Chromium”：WebView2 用系统的，安装包只剩 Rust 二进制和前端资源；
内存上主进程从 80 MB 降到 14 MB，整棵树因为 WebView2 同样走多进程，差距缩小但仍低约 27%。

真机同步一致性（同一台手机、同一时间窗、双方都从空缓存各跑一次完整同步）：

```
gt5: 6 messages on both sides
gt5: 6 messages compared field by field   (含 slot / carrier / simNumber 双卡字段)
gt5: SIM labels ["0|中国电信|+8619973712132","1|中国电信|+8617722562609"]
gt5: 4 conversations identical
RESULT: caches are equivalent
```

## 与旧 Electron 版的行为差异（有意为之）

1. **修复“删掉的手机重启后又回来”**：旧 `migrate.js` 每次启动都把 `SMS Glass` 老 profile
   里缺失的手机合并回来，删除无效（还会把清掉的缓存补回来）。新实现加了两个兼容键：
   `legacyMigrated`（每个老 profile 只迁移一次）和 `removedPhoneIds`（删除墓碑），
   并且只在目标 profile 为空时才做老 profile 恢复。JSON 格式没变，旧客户端会忽略这两个键。
2. **IPC 命令名映射**：渲染器看到的通道名仍是 `state:get` / `window:toggle-maximize` 等
   22 个原名；Tauri 由 Rust 函数名派生命令名，因此 `web/ui/tauri.js` 里做了机械映射
   （`:`/`-` → `_`，一通道一命令），`scripts/check-shim.cjs` 逐条断言。
3. 根证书校验用 rustls 自定义 verifier：逐字节比较证书 DER（比 CA 链校验更严格），
   不做主机名匹配（手机证书签给 LAN IP，旧实现同样关掉了 hostname 校验）。
4. 标题栏拖拽改由 `tauri.js` 调 `startDragging()`（WebView2 不认 `-webkit-app-region`）。
5. UI 缩放改用 WebView2 的 zoom factor（`webview.set_zoom`），行为与 Electron 的
   `setZoomFactor` 一致。
6. 安装包文件名由 `scripts/rename-installer.cjs` 改成旧约定（Tauri 不支持配置该项），
   `npm run dist` 已自动执行。
7. 更新检查完全照旧：只认 GitHub release 的 `digest`，镜像只用于下载。当前最新 release
   （v2.2.0）只带 APK、没有 Windows 安装包，因此现在不会提示更新——这与旧客户端行为一致。


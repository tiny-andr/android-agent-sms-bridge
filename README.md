# 短信桥 · SmsBridge

把手机短信变成 Agent 能直接查的本地 HTTP 接口。查数据库在请求时按需发生，没有任何轮询定时器。

## 功能

- **结构化查询**：按 天数 / 号码 / 卡槽（双卡）/ 关键词 / 收件箱或已发送 过滤，返回 JSON
- **OTP 模式**：`purpose=otp` 只返回验证码 + 掩码正文，正文里的其他数字全部打码
- **增量事件**：ContentObserver 监听新短信实时入队，Agent 用 `since_seq` 拉取，不重复不漏
- **后台常驻**：`specialUse` 前台服务 + 15 分钟兜底闹钟 + 开机自启，锁屏、划掉界面后仍可查询
- **配对与审计**：6 位一次性码配对，手机端可查看/吊销每个 Agent，带请求审计和限流
- **只读**：不发短信、不写短信库、不复制短信到本地，只读系统 Provider

## 快速开始

1. 安装：`adb install -r app\build\outputs\apk\debug\app-debug.apk`
2. 打开 App → 打开"启用服务" → 授予短信 / 电话状态 / 通知权限
3. **realme UI 必做两步**（实测：不做这两步，锁屏 40 秒就被「应用速冻」冻死，前台服务也救不了）：
   - 设置 → 应用管理 → 短信桥 → 耗电行为选「完全允许后台行为」、允许自启动
   - **关闭省电模式**
   做完后锁屏 5 分钟以上仍可正常查询。App 内另有 20 秒心跳开关（默认开）兜底其他 ROM。
4. 进"配对管理"，拿到 6 位码（5 分钟有效）
5. PC 端配对：

```powershell
python mcp\smsctl.py pair --url http://192.168.1.100:8787 --code 123456 --name DSH-PC
```

6. 查询：

```powershell
python mcp\smsctl.py search --days 3 --limit 10
python mcp\smsctl.py otp --address 10086 --days 1
python mcp\smsctl.py events --since 0
```

## Agent 接入

- **REST**：接口与签名见 [API.md](API.md)
- **MCP**：`python mcp\sms_mcp_server.py`（stdio transport，mcp>=2.0）
  工具：`sms_health` `sms_search` `sms_latest` `sms_otp` `sms_events` `sms_stats` `sms_pair`

## 构建

需要 JDK 17+ / Android SDK 36 / Gradle 8.13。完整步骤（含 Android Studio 与命令行两种）见
[BUILD.md](BUILD.md)，最短路径：

```bash
echo "sdk.dir=/path/to/Android/Sdk" > local.properties
gradle assembleDebug   # -> app/build/outputs/apk/debug/app-debug.apk
```

> **给 AI Agent**：如果机器上没有 Android 构建链，**自行安装**，不要问用户 ——
> Temurin JDK 21 + Android command-line tools（`sdkmanager "platforms;android-36" "build-tools;36.0.0" "platform-tools"`）
> + Gradle 8.13，然后写 `local.properties` 的 `sdk.dir`，再 `gradle assembleDebug`。
> 首次构建约下载 700 MB 依赖到 `~/.gradle/caches`（跨项目复用）。详见 [BUILD.md](BUILD.md)。

## 多语言

界面支持 **English / 简体中文 / 日本語**，主界面右上角按钮切换（同时注册了系统级 per-app
language，`设置 → 应用 → 本应用 → 语言` 也能改）。默认跟随系统，缺省语言是英文。

## 保活语义（unless-stopped）

`serviceEnabled` 记录的就是你的意图：

- 点「启用服务」→ true → 之后重启、被系统杀、划掉界面，都会自动恢复
- 点「停止」→ false → 不再自动拉起

四路恢复：开机/应用更新广播、15 分钟 AlarmManager、15 分钟 JobScheduler（persisted）、每次打开 App。

**唯一例外**：系统设置里的「强行停止」。Android 会把应用置为 stopped，广播/闹钟/Job 全部失效，必须手动再打开一次 App —— 这是系统级限制，无 root 绕不过。

## 安全模型

- 每个 Agent 独立 token（32 字节随机），请求用 HMAC-SHA256 签名，带 ±5 分钟时间窗 + nonce 防重放
- 手机端硬上限：最大可查天数、单次条数、限流、屏蔽号码 —— Agent 改不了
- 令牌明文只存在 App 私有目录（未 root 设备其他应用读不到），吊销立即生效
- 局域网内是明文 HTTP（签名保证完整性与身份），跨网访问请走 Tailscale / WireGuard 隧道

## 许可证

MIT

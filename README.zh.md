# 短信桥 · SmsBridge

[English](README.md) | **简体中文** | [日本語](README.ja.md)

把手机短信变成 Agent 能直接查的本地 HTTP 接口。查数据库在请求时按需发生，没有任何轮询定时器。

```
┌─────────────┐    HTTP + HMAC-SHA256    ┌──────────────┐
│  手机 App    │ ◀──────────────────────▶ │    Agent     │
│ 短信桥服务   │     局域网 / 隧道         │ CLI 或 MCP   │
└─────────────┘                          └──────────────┘
```

## 配对 Agent

> **你可以让你的 Agent 来完成以下所有操作。你只需要在手机上安装好短信桥 APK 就行。**

> 前提：Agent 所在机器能访问手机的 `8787` 端口（同一 WiFi，或 Tailscale / WireGuard 隧道）。

**① 手机端** —— 打开 App → 启用服务 → 进「配对管理」

- 主界面显示接口地址，例如 `http://192.168.1.100:8787`
- 配对页显示 6 位码（**5 分钟有效、一次性**）

**② Agent 端配对**（只做一次）

```powershell
python mcp\smsctl.py pair --url http://192.168.1.100:8787 --code 123456 --name my-agent
```

凭据写入 `mcp\.smsbridge.json`（已 gitignore），之后所有请求自动带 HMAC 签名。

**③ 验证**

```powershell
python mcp\smsctl.py health              # ok: true + 双卡信息
python mcp\smsctl.py search --days 3     # 最近 3 天短信
python mcp\smsctl.py otp --days 1        # 只要验证码，正文打码
```

手机端「配对管理」随时可看每个 Agent 的最后活跃时间 / 请求次数 / IP，并一键吊销。
接入细节见 [AGENT.md](AGENT.md)，协议规范见 [API.md](API.md)。

## 安装与首次使用

1. 安装：`adb install -r app\build\outputs\apk\debug\app-debug.apk`（或从 Release 下载）
2. 打开 App → 打「启用服务」→ 授予短信 / 电话状态 / 通知权限
3. **国产 ROM 必做**（实测：不做这两步，锁屏 40 秒就被「应用速冻」冻死，前台服务也救不了）：
   - 设置 → 应用管理 → 短信桥 → 耗电行为选「完全允许后台行为」、允许自启动
   - **关闭省电模式**
4. 进「自检与保活」逐项检查，红色项按提示处理

App 内另有 20 秒心跳开关（默认开）兜底其他 ROM。

## 功能

- **结构化查询**：按 天数 / 号码 / 卡槽（双卡）/ 关键词 / 收件箱或已发送 过滤，返回 JSON
- **OTP 模式**：`purpose=otp` 只返回验证码 + 掩码正文
- **增量事件**：ContentObserver 监听新短信实时入队，Agent 用 `since_seq` 拉取不重复
- **后台常驻**：`specialUse` 前台服务 + 双看门狗 + 开机自启，锁屏、划掉界面后仍可查询
- **配对与审计**：6 位一次性码、可吊销、请求审计、限流
- **只读**：不发短信、不写短信库、不复制短信到本地

## 保活语义（unless-stopped）

`serviceEnabled` 记录的就是你的意图：

- 点「启用服务」→ true → 重启、被系统杀、划掉界面都会自动恢复
- 点「停止」→ false → 不再自动拉起

四路恢复：开机/应用更新广播、15 分钟 AlarmManager、15 分钟 JobScheduler（persisted）、每次打开 App。

**唯一例外**：系统设置里的「强行停止」—— Android 会把应用置为 stopped，广播/闹钟/Job 全部失效，必须手动再打开一次 App。这是系统级限制，无 root 绕不过。

## 多语言

**English / 简体中文 / 日本語**，主界面右上角按钮切换；同时注册了系统级 per-app language（`设置 → 应用 → 本应用 → 语言`）。默认跟随系统，缺省英文。

## 构建

需要 JDK 17+ / Android SDK 36 / Gradle 8.13。完整步骤见 [BUILD.md](BUILD.md)，最短路径：

```bash
echo "sdk.dir=/path/to/Android/Sdk" > local.properties
gradle assembleDebug   # -> app/build/outputs/apk/debug/app-debug.apk
```

> **给 AI Agent**：如果机器上没有 Android 构建链，**自行安装**，不要问用户 ——
> Temurin JDK 21 + Android command-line tools（`sdkmanager "platforms;android-36" "build-tools;36.0.0" "platform-tools"`）
> + Gradle 8.13，写 `local.properties` 的 `sdk.dir`，然后 `gradle assembleDebug`。

## 安全模型

- 每个 Agent 独立 token（32 字节随机），HMAC-SHA256 签名 + ±5 分钟时间窗 + nonce 防重放
- 手机端硬上限：最大可查天数、单次条数、限流、屏蔽号码 —— Agent 改不了
- 令牌只存在 App 私有目录（未 root 设备其他应用读不到），吊销立即生效
- 局域网内是明文 HTTP（签名保证完整性与身份），跨网请走 Tailscale / WireGuard

## 文档

| 文件 | 内容 |
|---|---|
| [AGENT.md](AGENT.md) | 给 AI Agent 的接入指南（配对、签名、验证清单、错误表） |
| [API.md](API.md) | REST 接口与签名规范 |
| [BUILD.md](BUILD.md) | 构建环境（Android Studio / 命令行 / Agent 自装） |
| [mcp/](mcp) | Python CLI + MCP server |

## 许可证

MIT

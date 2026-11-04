# SMS Bridge · SmsBridge

[English](README.md) | [简体中文](README.zh.md) | **日本語**

スマートフォンの SMS を、エージェントが問い合わせられるローカル HTTP API に変えます。
SMS データベースはリクエストが来たときだけ参照され、ポーリングループはありません。

```
┌─────────────┐    HTTP + HMAC-SHA256    ┌──────────────┐
│ スマホアプリ │ ◀──────────────────────▶ │    エージェント │
│ SMS Bridge  │     LAN / トンネル        │ CLI または MCP │
└─────────────┘                          └──────────────┘
```

## エージェントのペアリング

> **以下の操作はすべてエージェントに任せられます。必要なのはスマートフォンに SMS Bridge の APK をインストールしておくことだけです。**

> 前提：エージェント側のマシンからスマートフォンの `8787` ポートに到達できること
> （同じ Wi-Fi、または Tailscale / WireGuard などのトンネル）。

**① スマートフォン側** —— アプリを開く → 「サービスを有効化」→ 「ペアリング」を開く

- メイン画面にエンドポイントが表示されます（例：`http://192.168.1.100:8787`）
- ペアリング画面に 6 桁のコードが表示されます（**5 分間有効、1 回限り**）

**② エージェント側でペアリング**（1 回だけ）

```powershell
python mcp\smsctl.py pair --url http://192.168.1.100:8787 --code 123456 --name my-agent
```

認証情報は `mcp\.smsbridge.json` に保存され（gitignore 済み）、以降のリクエストは自動的に HMAC 署名されます。

**③ 動作確認**

```powershell
python mcp\smsctl.py health              # ok: true + 2 枚の SIM 情報
python mcp\smsctl.py search --days 3     # 直近 3 日間の SMS
python mcp\smsctl.py otp --days 1        # 認証コードのみ、本文はマスク
```

スマートフォン側の「ペアリング」画面では、各エージェントの最終アクセス時刻・リクエスト数・IP を確認でき、
即座に無効化できます。連携の詳細は [AGENT.md](AGENT.md)、プロトコル仕様は [API.md](API.md)。

## インストールと初回起動

1. インストール：`adb install -r app\build\outputs\apk\debug\app-debug.apk`（または Release から）
2. アプリを開く → 「サービスを有効化」→ SMS / 電話状態 / 通知の権限を許可
3. **中国メーカー ROM では必須**（実測：この 2 つをやらないと、画面オフから 40 秒で
   「アプリ凍結」され、フォアグラウンドサービスでも復帰できません）：
   - 設定 → アプリ管理 → SMS Bridge → 電池動作を「バックグラウンド動作をすべて許可」にし、自動起動を許可
   - **省電力モードをオフ**
4. 「セルフチェックと常駐」で各項目を確認し、赤い項目を指示どおりに処理

アプリ内には 20 秒のハートビート設定（既定でオン）があり、他の ROM でも保険になります。

## 機能

- **構造化クエリ**：日数 / 送信元 / SIM スロット（デュアル SIM）/ キーワード / 受信・送信で絞り込み、JSON で返却
- **OTP モード**：`purpose=otp` は認証コードとマスク済み本文だけを返す
- **増分イベント**：ContentObserver が新着 SMS をリアルタイムにキューへ。`since_seq` で重複なく取得
- **常駐**：`specialUse` フォアグラウンドサービス + 2 系統のウォッチドッグ + 起動時自動開始。
  画面ロック中やタスクをスワイプしても応答し続けます
- **ペアリングと監査**：1 回限りの 6 桁コード、クライアントの無効化、監査ログ、レート制限
- **読み取り専用**：SMS の送信・DB への書き込み・ローカルへの複製は一切しません

## 常駐の意味（unless-stopped）

`serviceEnabled` がユーザーの意思を表します：

- 「サービスを有効化」→ true → 再起動・OS による強制終了・タスク除去の後も自動復帰
- 「停止」→ false → 二度と自動起動しない

復帰経路は 4 系統：起動 / アプリ更新ブロードキャスト、15 分間隔の AlarmManager、
15 分間隔の JobScheduler（persisted）、アプリを開くたび。

**唯一の例外**：システム設定の「強制停止」。Android はアプリを stopped 状態にし、
ブロードキャスト・アラーム・ジョブがすべて無効になります。手動でアプリを開き直すまで復帰しません。
これはシステムレベルの制限で、root なしでは回避できません。

## 多言語

UI は **English / 简体中文 / 日本語** に対応。メイン画面の右上ボタンで切り替えられます。
システムの per-app language（`設定 → アプリ → SMS Bridge → 言語`）にも登録済み。
既定はシステムに従い、該当なしの場合は英語になります。

## ビルド

JDK 17+ / Android SDK 36 / Gradle 8.13 が必要です。詳細は [BUILD.md](BUILD.md)、最短手順：

```bash
echo "sdk.dir=/path/to/Android/Sdk" > local.properties
gradle assembleDebug   # -> app/build/outputs/apk/debug/app-debug.apk
```

> **AI エージェントへ**：マシンに Android のビルド環境がない場合は、ユーザーに聞かずに
> **自分で導入**してください —— Temurin JDK 21 + Android command-line tools
> （`sdkmanager "platforms;android-36" "build-tools;36.0.0" "platform-tools"`）+ Gradle 8.13 を入れ、
> `local.properties` に `sdk.dir` を書き、`gradle assembleDebug` を実行します。

## セキュリティモデル

- エージェントごとに独立したトークン（32 バイトの乱数）。HMAC-SHA256 署名 + 前後 5 分のタイムウィンドウ + nonce によるリプレイ防止
- スマートフォン側の上限（最大日数・最大件数・レート制限・ブロック番号）はエージェントから変更不可
- トークンはアプリのプライベート領域にのみ保存（未 root 端末では他アプリから読めません）。無効化は即時反映
- LAN 内は平文 HTTP（署名が完全性と身元を保証）。ネットワークをまたぐ場合は Tailscale / WireGuard を使用

## ドキュメント

| ファイル | 内容 |
|---|---|
| [AGENT.md](AGENT.md) | AI エージェント向け連携ガイド（ペアリング、署名、確認手順、エラー表） |
| [API.md](API.md) | REST API と署名仕様 |
| [BUILD.md](BUILD.md) | ビルド環境（Android Studio / コマンドライン / エージェントによる自動導入） |
| [mcp/](mcp) | Python CLI + MCP サーバー |

## ライセンス

MIT

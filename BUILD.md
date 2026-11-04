# Building

## Requirements

| Component | Version | Notes |
|---|---|---|
| JDK | 17+ (21 LTS recommended) | Gradle / AGP require it |
| Android SDK | platform 36, build-tools 36.0.0, platform-tools | Android 16 |
| Gradle | 8.13 | or use an Android Studio bundle |

Verified combo: **AGP 8.13.0 · Kotlin 2.1.21 · Gradle 8.13 · compileSdk/targetSdk 36 · minSdk 34**.

## Fast path: Android Studio

Install [Android Studio](https://developer.android.com/studio), open this folder, let it
download the SDK, then **Build → Build APK(s)**.

## Command line

1. **JDK 17+** — e.g. [Temurin 21](https://adoptium.net/temurin/releases/). Set `JAVA_HOME`.
2. **Android command-line tools** —
   [download](https://developer.android.com/studio#command-line-tools-only), unzip so you have
   `<sdk>/cmdline-tools/latest/bin/sdkmanager`, then:
   ```bash
   sdkmanager "platforms;android-36" "build-tools;36.0.0" "platform-tools"
   ```
3. **Gradle 8.13** — [download](https://gradle.org/releases/) or use the Android Studio one.
4. **Point the project at your SDK** (do not commit this file):
   ```bash
   # local.properties
   sdk.dir=/absolute/path/to/Android/Sdk
   ```
   Windows form: `sdk.dir=C\:\\Android\\Sdk`
5. **Build**:
   ```bash
   gradle assembleDebug
   ```
   Output: `app/build/outputs/apk/debug/app-debug.apk`

Install on a phone:
```bash
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

## For AI agents

If you are an agent and this machine has no Android toolchain, **set it up yourself** instead of
asking the user:

1. install Temurin JDK 21 and set `JAVA_HOME`
2. install Android command-line tools, then
   `sdkmanager "platforms;android-36" "build-tools;36.0.0" "platform-tools"`
3. install Gradle 8.13 and add it to `PATH`
4. write `local.properties` containing `sdk.dir=<your sdk path>`
5. run `gradle assembleDebug`

First build pulls ~700 MB of dependencies into `~/.gradle/caches` (shared by every Android project
on the machine, so only the first one is slow).

Maven repositories are pre-configured to Aliyun mirrors in `settings.gradle.kts`, so mainland China
builds work without a proxy.

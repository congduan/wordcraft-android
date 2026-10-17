# 任务：为 WordCraft 搭建 Android 壳工程（方案 A：eframe + android-activity 直打 APK）

## 背景
- 本仓库是 storytold/wordcraft（纯 Rust 分层架构）的 fork，目标是 Android 移植（方案 A：直接把 egui 前端用 eframe + android-activity 打包，先跑起来，不求触屏体验完美）。
- 上游桌面壳在 `apps/wordcraft`（eframe 0.36 + wgpu），浏览器壳在 `apps/wordcraft-web`（eframe WebRunner + wgpu，`src/web.rs` 的 `start()` 是现成参考——`WordApp::new(Session::new(...), Services)` + `Services::default()` 即可跑，所有 Services 字段都是 Option，可全部 None）。
- `crates/ui-egui` 的 `WordApp` 实现了 `eframe::App`，入口 `WordApp::new(session, services)`；文档用 `wordcraft_engine::blank_document(&region)` 或 `sample_document()`。
- workspace `Cargo.toml` 有 `[workspace.lints]`（unsafe_code = "forbid"）与 `[profile.*]`，新成员加入 workspace members 即继承。

## 环境（已装好，直接用，勿重装）
- `ANDROID_HOME=/opt/android-sdk`（cmdline-tools/latest、platforms;android-34、build-tools;34.0.0、platform-tools、ndk;27.2.12479018）
- JDK 17（`java -version` 可验），gradle 用 gradle-wrapper（版本自选 8.x）
- rustup targets 已装：`aarch64-linux-android`、`armv7-linux-androideabi`
- `cargo-ndk` 未装，可自行 `cargo install cargo-ndk --locked`（或用 cargo-gradle 插件链 cargo-ndk）
- 本机 2 核 2GB 内存 + 6GB swap，**一次只跑一个 cargo/gradle 命令，严禁并行编译**；gradle 的 JVM 内存参数要设小（org.gradle.jvmargs=-Xmx1024m）

## 任务分解（按序）
1. 新建 `apps/wordcraft-android/`：
   - Rust 侧：新 bin crate `wordcraft-android`，加入 workspace members。代码参考 `apps/wordcraft-web/src/web.rs` 的装配方式：`eframe::run_native` + `NativeOptions`，**Android target 时 renderer 用 wgpu**（eframe 0.36 在 android 上需 `eframe` 的 `android-native-activity` feature 或 winit 的 android-native-activity；自行确认 0.36 的正确接线，关键点：`android_main` 入口由 `eframe::run_native` 的 winit android 支持，`#[cfg(target_os = "android")]` 下用 `#[no_mangle] fn android_main`）。注意 workspace 禁 unsafe、禁 unwrap/expect/panic——遵守上游 AGENTS.md 的 never-crash 规范，错误一律 log + 优雅退出。
   - Android 侧：gradle 工程（`app/` 模块，minSdk 24，targetSdk 34，abiFilters arm64-v8a + armeabi-v7a），用 `cargo-ndk` 在 gradle task 里交叉编译 Rust（推荐 cargo-ndk-android-gradle 模式：`tasks.register('buildRust')` Exec task 调 `cargo ndk -t arm64-v8a -t armeabi-v7a -o app/src/main/jniLibs build --release`，挂到 `preBuild` dependsOn）。加载库名 `libwordcraft_android.so`，MainActivity 直接继承 `android.app.NativeActivity`（meta-data android.app.lib_name = wordcraft_android）。manifest 里 NativeActivity 要加 `android:hardwareAccelerated="true"` 与软键盘相关配置（`android:windowSoftInputMode="adjustResize"`）。
   - 应用 icon 用简单的自绘/纯色 launcher icon（上游规定不许用任何 Microsoft/Word 图标素材，也不许引入有归属问题的素材——用代码生成或公共领域素材）。
2. **裁剪**：desktop 壳依赖（clipboard/arboard、file_dialogs/rfd、control_server、printing、apple_events 等）在 android 上不可用，一律 `#[cfg(not(target_os = "android"))]` 隔离或干脆在 android 壳里只装配 `Services::default()` + blank/sample 文档，不引入 apps/wordcraft 的桌面模块。目标是**编译通过并出包**，功能裁剪在代码注释里记录。
3. 构建出 debug APK：`./gradlew assembleDebug`（gradle 内存参数见上；swap 已有 6G，若 OOM 就把 cargo build 并到 gradle 外先手动跑再让 gradle 只打包——允许用 `-Pandroid.prebuilt=true` 之类的开关自行设计，只要 `./gradlew assembleDebug` 全流程可复现）。
4. 产物命名 `wordcraft-android-debug.apk` 放仓库根 `dist/`（gitignore 掉 dist/，APK 不入库，但提交说明里写清产物路径）。
5. git 提交（不要 push，主控验收后统一推）。提交信息前缀 `android-port:`，勿加 Co-Authored-By。提交作者按仓库现有 git config。

## 验证标准（完成前自查，输出贴原始结果）
- `cargo check -p wordcraft-android --target aarch64-linux-android` 通过（贴尾部输出）
- `./gradlew assembleDebug` 通过，`app/build/outputs/apk/debug/app-debug.apk` 存在（贴 `ls -la` 与 `sha256sum`）
- `aapt dump badging app-debug.apk | head -8` 贴出（确认 package 名、NativeActivity、abi）
- APK 内含 `lib/arm64-v8a/libwordcraft_android.so`（`unzip -l` 贴证据）

## 硬约束
- 只在 `apps/wordcraft-android/`、workspace `Cargo.toml` 的 members 行内做改动；**严禁改 crates/ 下任何上游代码**（若发现必须改上游才能编译，停下来在报告中说明原因与最小 patch 建议，勿直接改）
- 一次只跑一个 cargo/gradle 命令（2GB 内存机器，并行编译会 OOM 整机）
- 单条构建命令 timeout 上限 30 分钟；cargo release 构建若超时，可降级让 APK 只带 arm64-v8a 一个 ABI
- 假设可证伪：简报中任何关于 eframe 0.36 android 接线的描述若与实际 crate 文档/源码冲突，以实际为准并在报告中记录差异
- 完成标准：上述验证四件全绿即 git commit；不要为跑通全部功能而扩大范围——本刀交付物是「能安装启动的 debug APK 壳」，触屏交互优化与 IME 接入是后续刀

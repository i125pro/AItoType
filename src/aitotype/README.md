# AItoType（egui 原生版）

这是一个**已彻底移除 Tauri / WebView2** 的分支：界面不再由系统 WebView2 渲染，
而是用 Rust 的 [egui](https://github.com/emilk/egui)（eframe）直接绘制到原生窗口。

因此：

- Windows 上**不再启动 `msedgewebview2.exe`**，常驻内存从数百 MB 降到几十 MB 量级；
- 运行**不依赖系统 WebView2 Runtime**，无需安装任何运行库；
- 后端（录音 / 语音识别 / 键盘模拟 / 易错词 / 日志）为纯 Rust，全部保留。

## 目录结构

```
src/aitotype/src-tauri/
├── Cargo.toml
├── icons/                 # 图标（托盘使用 32x32.png）
└── src/
    ├── main.rs            # 程序入口（eframe::run_native）
    ├── app.rs             # egui 界面 + 业务编排
    ├── paths.rs           # 配置 / 日志目录（替代 tauri path API）
    ├── config.rs          # STT 配置读写与规范化
    ├── settings.rs        # 界面级设置（快捷键 / 自动复制）
    ├── hotkey.rs          # 全局快捷键（global-hotkey）
    ├── tray.rs            # 系统托盘（tray-icon）
    ├── fonts.rs           # 中文字体加载
    ├── audio.rs           # 录音（cpal / hound）
    ├── stt.rs             # 语音识别（OpenRouter / SiliconFlow）
    ├── keyboard.rs        # 键盘模拟 / 剪贴板（enigo / arboard）
    ├── corrections.rs     # 易错词记忆与替换
    └── logging.rs         # 转写日志
```

## 开发与构建

前置要求：Rust 稳定版（`rustup`）。

```bash
cd src/aitotype/src-tauri
cargo run --release       # 运行
cargo build --release     # 构建
```

产物：`target/release/aitotype`（Windows 为 `aitotype.exe`）。

Linux 额外依赖：

编译期：

```bash
sudo apt-get install -y pkg-config libasound2-dev libgtk-3-dev \
  libayatana-appindicator3-dev libxdo-dev libxkbcommon-dev libx11-dev \
  libssl-dev
```

运行期（缺 `libxkbcommon-x11` 会导致启动 panic）：

```bash
sudo apt-get install -y libxkbcommon-x11-0 libxkbcommon0 libgl1 libegl1 \
  libwayland-client0 libwayland-egl1
```

## 说明

- 配置保存在系统配置目录下的 `AItoType/`（`config.json`、`ui.json`、`corrections.json`）。
- 转写日志保存在系统数据目录下的 `AItoType/logs/`。
- 托盘图标菜单：Show Window / Quit AItoType。
- Linux：暂不启用系统托盘（`tray-icon` 需 GTK 主循环），关闭窗口即退出程序。
- 已验证：在 Linux（Debian 13 / x86_64，cargo 1.99）上成功完成 `cargo build --release`（0 error）并可正常启动运行。
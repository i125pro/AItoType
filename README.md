# AItoType 🎙️

<p align="center">
  <img src="docs/screenshots/app-icon.png" alt="AItoType Icon" width="128" height="128" />
</p>

AItoType 是一个开源、免费的桌面端语音转文字工具，基于 **Rust + egui** 构建，旨在提供极致轻量、高效的语音输入体验。

> 本版本已**彻底移除 Tauri / WebView2**：界面由 egui 直接渲染到原生窗口，Windows 上不再启动 `msedgewebview2.exe`，也不依赖系统 WebView2 Runtime，常驻内存显著降低。

**核心理念**：免费开源、极简设计、高效生产力。

> ⚠️ **声明**：本项目完全开源免费，供大家学习与使用。**严禁用于任何商业盈利行为**。

---

## 📖 TLDR.

- **首次配置**：首次打开需进入配置界面，分两步完成：(1) 选择供应商；(2) 填入 API Key。
- **界面交互**：点击主界面“录音”按钮开始，再次点击停止。识别结果会显示在主界面并可一键复制。
- **后台/快捷键模式**：支持关闭主窗口后台运行。通过预设快捷键唤醒录制，再次按键停止。识别完成后会自动写入当前光标所在输入框（自动粘贴）。
- **历史记录**：History 页面按条目卡片展示，每条可点击直接复制内容。
- **权限引导**：首次使用会引导完成麦克风与辅助功能授权，确保可录音和后台自动写入。
- **供应商**：目前支持 OpenRouter 和 [SiliconFlow](https://cloud.siliconflow.cn/i/zbtzfOmG) 。OpenRouter 默认使用 Gemini 3.0 Flash，[SiliconFlow](https://cloud.siliconflow.cn/i/zbtzfOmG) 默认使用 TeleAI 模型。**划重点，现在硅基流动的 TeleAI/TeleSpeechASR 完全免费，速度快且随便用。**

## 📸 界面预览

> 界面已由 egui 重绘，以下截图来自旧版（Tauri/WebView）界面，仅示意功能布局。

<p align="center">
  <img src="docs/screenshots/main.png" alt="主界面" width="45%" border="1" />
  <img src="docs/screenshots/setting.png" alt="设置页" width="45%" border="1" />
</p>

## ✨ 功能特性

- **全局快捷键唤起**：默认 `Alt+Space`（Windows 默认 `Ctrl+Shift+Space`）一键录音，再次按下即可停止并识别。
- **无缝嵌入工作流**：
  - **自动写入（自动粘贴）**：识别完成后，结果自动“打字”到你当前光标所在的输入框（Chatbox、文档、编辑器等）。
  - **自动复制**：可选自动复制结果到剪贴板。
- **历史记录优化**：历史页面按条目卡片展示，点击任意卡片可立即复制该条内容。
- **灵活的模型支持**：
  - 内置 **OpenRouter** 支持。
  - 默认配置 **Gemini 3.0 Flash**（速度快、精度高、免费额度充裕）。
  - 可自定义 API Key 和 Model（如 GPT-4o, Claude 3.5 Sonnet 等）。
- **隐私安全**：音频数据仅在您的设备上录制，并直接发送至您配置的 API 服务商；本地不留存录音音频文件。

## 🛠️ 技术栈

- **Core**: [Rust](https://www.rust-lang.org/)（egui / eframe、cpal、hound、arboard、enigo、global-hotkey、tray-icon）
- **GUI**: egui（立即模式原生 UI，**无 WebView / WebView2**）
- **API**: OpenRouter / SiliconFlow（兼容 OpenAI 格式）

## 🚀 快速开始

### 前置要求

- macOS / Windows / Linux
- Rust 稳定版（推荐通过 `rustup` 安装）

### 开发运行

```bash
# 克隆项目
git clone https://github.com/i125pro/AItoType.git
cd AItoType/src/aitotype/src-tauri

# 运行（开发）
cargo run

# 运行（发布）
cargo run --release
```

### 打包构建

```bash
cd src/aitotype/src-tauri
cargo build --release
```

构建产物位于 `src/aitotype/src-tauri/target/release/`：

- macOS / Linux：`aitotype`
- Windows：`aitotype.exe`

> 若需要 `.app` / `.dmg` / `.msi` 等平台安装包，可在此二进制基础上配合 `cargo-bundle`、`create-dmg`、`wix` 等工具打包。

### Linux 额外依赖

```bash
sudo apt-get install -y pkg-config libgtk-3-dev libx11-dev libxdo-dev \
  libasound2-dev libayatana-appindicator3-dev libxkbcommon-dev
```

## ⚙️ 配置指南

首次启动后，请点击主界面底部的 **设置 (⚙️)** 进行配置：

1. **Provider**: 选择 OpenRouter 或 SiliconFlow。
2. **API Key**: 填入对应平台的 Key（[OpenRouter.ai](https://openrouter.ai/) 或 [SiliconFlow](https://cloud.siliconflow.cn/i/zbtzfOmG)）。
3. **Model**: OpenRouter 默认 `google/gemini-3-flash-preview`，SiliconFlow 默认使用 TeleAI 模型。
4. **Global Shortcut**: 点击录制你习惯的快捷键（如 `Cmd+M` 或 `F1`）。
5. **Auto-Copy**: 开启后，识别结果会自动进入剪贴板。

**注意**：配置会自动保存到本地，重启应用无需重新输入。

## 📂 本地文件位置

- 配置：系统配置目录下 `AItoType/`（`config.json`、`ui.json`、`corrections.json`）
- 转写日志：系统数据目录下 `AItoType/logs/`（每日一个 `transcribe_YYYY-MM-DD.json`）

## 🔐 权限引导（macOS）

为保证“录音 + 后台自动写入”正常工作，首次使用建议按以下顺序完成授权：

1. **麦克风权限**：允许 AItoType 访问麦克风（系统会弹窗）。
2. **辅助功能权限**：前往 `系统设置 -> 隐私与安全性 -> 辅助功能`，勾选 AItoType。
3. 若权限刚修改，建议重启一次应用再测试快捷键录音与自动写入。

## 🔑 API Key 安全说明

- 你的 API Key 会保存在**本机应用配置目录**，不会写入本仓库代码。
- 当前实现为本地明文保存，建议使用低权限/限额的 API Key 并定期轮换。

## 🤝 贡献与反馈

欢迎提交 Issue 或 Pull Request！

## 📄 许可证 (License)

本项目采用 **CC BY-NC-SA 4.0** 协议（署名-非商业性使用-相同方式共享）。

- ✅ **允许**：复制、分发、修改、二次创作（需署名，并以相同协议共享）。
- ❌ **禁止**：将本项目或其衍生作品用于商业用途。

协议详情：

- License summary: https://creativecommons.org/licenses/by-nc-sa/4.0/
- Legal code: https://creativecommons.org/licenses/by-nc-sa/4.0/legalcode

---

Made with ❤️ by Robin. Enjoy typing with your voice!
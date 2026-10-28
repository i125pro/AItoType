// AItoType - 轻量级语音转文字工具（egui 原生版）
//
// 已彻底移除 Tauri / WebView / WebView2：界面由 egui 直接渲染。
// Windows release 下不额外弹出控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod corrections;
mod fonts;
mod hotkey;
mod keyboard;
mod logging;
mod paths;
mod settings;
mod stt;
mod tray;

fn main() -> eframe::Result<()> {
    // 加载 .env（用于 OPENROUTER_API_KEY / SILICONFLOW_API_KEY 等）
    let _ = dotenv::dotenv();

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("AItoType")
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([820.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "AItoType",
        native_options,
        Box::new(|cc| Ok(Box::new(app::AitoTypeApp::new(cc)))),
    )
}
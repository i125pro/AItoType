//! 系统托盘
//!
//! 关键设计：托盘菜单事件在**后台线程**直接处理，不依赖 GUI 主循环。
//! 主窗口隐藏后 eframe 不再触发 `update`，因此「退出」必须在这里直接完成，
//! 「显示窗口」则通过共享标记 + 请求重绘来唤醒界面。
#![allow(dead_code)]

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::core::SharedState;

/// 从内置 PNG 加载托盘图标
pub fn load_icon() -> Result<Icon, String> {
    let bytes = include_bytes!("../icons/32x32.png");
    let image = image::load_from_memory(bytes)
        .map_err(|e| format!("解码托盘图标失败: {:?}", e))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|e| format!("创建托盘图标失败: {:?}", e))
}

/// 创建托盘图标。返回的 TrayIcon 必须由调用方持有以维持生命周期。
pub fn build_tray(shared: SharedState, ctx: egui::Context) -> Result<TrayIcon, String> {
    let show_item = MenuItem::new("Show Window", true, None);
    let quit_item = MenuItem::new("Quit AItoType", true, None);
    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();

    let menu = Menu::new();
    menu.append_items(&[&show_item, &quit_item])
        .map_err(|e| format!("构建托盘菜单失败: {:?}", e))?;

    let icon = load_icon()?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("AItoType")
        .with_icon(icon)
        .build()
        .map_err(|e| format!("创建托盘失败: {:?}", e))?;

    std::thread::spawn(move || {
        let receiver = MenuEvent::receiver();
        while let Ok(event) = receiver.recv() {
            if event.id == show_id {
                if let Ok(mut state) = shared.lock() {
                    state.show_requested = true;
                }
                ctx.request_repaint();
            } else if event.id == quit_id {
                // 直接退出进程：不依赖 GUI 主循环，保证「退出」始终可用
                std::process::exit(0);
            }
        }
    });

    Ok(tray)
}
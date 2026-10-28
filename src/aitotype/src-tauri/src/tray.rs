//! 系统托盘
//!
//! 基于 `tray-icon`，替代原 Tauri 的 tray-icon 能力。
//! 菜单事件通过后台线程读取全局 channel 并转发到应用。
#![allow(dead_code)]

use std::sync::mpsc::Sender;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// 托盘向应用发送的指令
#[derive(Debug, Clone, Copy)]
pub enum TrayCommand {
    ShowWindow,
    Quit,
}

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
pub fn build_tray(tx: Sender<TrayCommand>) -> Result<TrayIcon, String> {
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
                let _ = tx.send(TrayCommand::ShowWindow);
            } else if event.id == quit_id {
                let _ = tx.send(TrayCommand::Quit);
            }
        }
    });

    Ok(tray)
}
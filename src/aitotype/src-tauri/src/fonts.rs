//! 中文字体加载
//!
//! egui 内置字体不含 CJK 字形，若不加载系统字体，中文会显示为方块。
//! 这里从常见系统路径探测一个中文字体并注册到 egui。

use eframe::egui;

#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    "C:/Windows/Fonts/msyh.ttc",
    "C:/Windows/Fonts/msyh.ttf",
    "C:/Windows/Fonts/simhei.ttf",
    "C:/Windows/Fonts/simsun.ttc",
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/Library/Fonts/Arial Unicode.ttf",
];

#[cfg(all(unix, not(target_os = "macos")))]
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    "/usr/share/fonts/truetype/arphic/uming.ttc",
];

fn read_first_available() -> Option<(String, Vec<u8>)> {
    for candidate in CANDIDATES {
        if let Ok(data) = std::fs::read(candidate) {
            let name = format!("cjk-{}", candidate);
            return Some((name, data));
        }
    }
    None
}

/// 安装中文字体（若探测成功）
pub fn install_fonts(ctx: &egui::Context) {
    let Some((name, data)) = read_first_available() else {
        eprintln!("未找到可用的中文字体，界面中文可能显示为方块。");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert(name.clone(), egui::FontData::from_owned(data));

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, name.clone());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push(name);

    ctx.set_fonts(fonts);
}
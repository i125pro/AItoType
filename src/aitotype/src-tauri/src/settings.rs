//! 界面级设置
//!
//! 原先散落在浏览器 localStorage 的项（快捷键、自动复制、跳过的更新版本），
//! 现在统一持久化到本地 `ui.json`。

use serde::{Deserialize, Serialize};

use crate::paths;

fn default_shortcut() -> String {
    #[cfg(target_os = "windows")]
    {
        "Ctrl+Shift+Space".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        "Alt+Space".to_string()
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    /// 全局快捷键字符串，如 "Alt+Space"
    #[serde(default = "default_shortcut")]
    pub shortcut: String,
    /// 识别后自动复制到剪贴板
    #[serde(default = "default_true")]
    pub auto_copy: bool,
    /// 用户选择跳过的版本号
    #[serde(default)]
    pub skipped_version: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            shortcut: default_shortcut(),
            auto_copy: true,
            skipped_version: String::new(),
        }
    }
}

pub fn load() -> UiSettings {
    let path = paths::ui_settings_file();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<UiSettings>(&content) {
                return settings;
            }
        }
    }
    UiSettings::default()
}

pub fn save(settings: &UiSettings) -> Result<(), String> {
    let path = paths::ui_settings_file();
    paths::ensure_parent(&path)?;
    let json =
        serde_json::to_string_pretty(settings).map_err(|e| format!("序列化设置失败: {:?}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("保存设置失败: {:?}", e))
}
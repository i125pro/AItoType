//! 应用路径解析
//!
//! 替代原先 Tauri 的 `app.path().app_config_dir()` / `app_log_dir()`。

use std::path::{Path, PathBuf};

/// 应用配置目录（跨平台）
pub fn app_config_dir() -> PathBuf {
    dirs::config_dir()
        .map(|dir| dir.join("AItoType"))
        .unwrap_or_else(|| PathBuf::from(".aitotype"))
}

/// 应用日志目录
pub fn app_log_dir() -> PathBuf {
    dirs::data_dir()
        .map(|dir| dir.join("AItoType").join("logs"))
        .unwrap_or_else(|| app_config_dir().join("logs"))
}

/// config.json 路径
pub fn config_file() -> PathBuf {
    app_config_dir().join("config.json")
}

/// corrections.json 路径
pub fn corrections_file() -> PathBuf {
    app_config_dir().join("corrections.json")
}

/// ui.json 路径（界面级设置：快捷键 / 自动复制等）
pub fn ui_settings_file() -> PathBuf {
    app_config_dir().join("ui.json")
}

/// 确保目标文件的父目录存在
pub fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {:?}: {:?}", parent, e))?;
    }
    Ok(())
}
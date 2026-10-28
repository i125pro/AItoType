//! STT 配置的加载、保存与规范化
//!
//! 逻辑迁移自原 lib.rs，去除 Tauri 依赖，改为基于 `paths` 模块读写文件。

use crate::paths;
use crate::stt::{self, SttConfig};

const LEGACY_ENHANCEMENT_OPENROUTER_MODEL: &str = "google/gemini-2.0-flash-001";
const LEGACY_ENHANCEMENT_PROGRAMMER_PROMPT: &str = "你是程序员语音转文字的润色助手。请按规则处理文本：\n1) 去除口头禅、重复词和无意义停顿词；\n2) 修正技术术语、产品名、代码相关拼写错误；\n3) 保留原意，不扩写、不总结、不补充新信息；\n4) 仅做必要标点与断句优化；\n5) 只输出润色后的最终文本，不要任何解释。\n\n原文：\n{text}";

/// 规范化配置（补全默认值、迁移遗留值）
pub fn normalize_stt_config(config: SttConfig) -> SttConfig {
    let mut normalized = config;
    normalized.provider = stt::normalize_provider(&normalized.provider);

    if normalized.base_url.trim().is_empty() {
        normalized.base_url = stt::default_base_url_for_provider(&normalized.provider).to_string();
    } else {
        normalized.base_url = normalized.base_url.trim().trim_end_matches('/').to_string();
    }

    if normalized.model.trim().is_empty() {
        normalized.model = stt::default_model_for_provider(&normalized.provider).to_string();
    } else {
        normalized.model = normalized.model.trim().to_string();
    }

    normalized.api_key = normalized.api_key.trim().to_string();
    normalized.enhancement_provider = stt::normalize_provider(&normalized.enhancement_provider);
    normalized.enhancement_base_url = normalized
        .enhancement_base_url
        .trim()
        .trim_end_matches('/')
        .to_string();
    normalized.enhancement_api_key = normalized.enhancement_api_key.trim().to_string();
    if normalized.enhancement_model.trim().is_empty() {
        normalized.enhancement_model =
            stt::default_enhancement_model_for_provider(&normalized.enhancement_provider)
                .to_string();
    } else if normalized.enhancement_provider == stt::PROVIDER_OPENROUTER
        && normalized
            .enhancement_model
            .trim()
            .eq_ignore_ascii_case(LEGACY_ENHANCEMENT_OPENROUTER_MODEL)
    {
        normalized.enhancement_model =
            stt::default_enhancement_model_for_provider(&normalized.enhancement_provider)
                .to_string();
    } else {
        normalized.enhancement_model = normalized.enhancement_model.trim().to_string();
    }
    if normalized.enhancement_prompt.trim().is_empty() {
        normalized.enhancement_prompt = SttConfig::default().enhancement_prompt;
    } else {
        let trimmed_prompt = normalized.enhancement_prompt.trim();
        if trimmed_prompt == LEGACY_ENHANCEMENT_PROGRAMMER_PROMPT {
            normalized.enhancement_prompt = SttConfig::default().enhancement_prompt;
        } else {
            normalized.enhancement_prompt = trimmed_prompt.to_string();
        }
    }
    normalized
}

/// 判断是否已解析出可用 API Key（含环境变量回退）
pub fn has_resolved_api_key(config: &SttConfig) -> bool {
    if !config.api_key.trim().is_empty() {
        return true;
    }

    let env_key = stt::env_key_for_provider(&config.provider);
    matches!(std::env::var(env_key), Ok(value) if !value.trim().is_empty())
}

/// 加载配置：默认值 -> 环境变量 -> 磁盘文件 -> 规范化
pub fn load_config() -> SttConfig {
    let mut config = SttConfig::default();
    let env_key = stt::env_key_for_provider(&config.provider);
    if let Ok(value) = std::env::var(env_key) {
        config.api_key = value.trim().to_string();
    }
    let env_api_key = config.api_key.clone();

    let path = paths::config_file();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(saved) = serde_json::from_str::<SttConfig>(&content) {
                config = saved;
            }
        }
    }

    if config.api_key.trim().is_empty() {
        config.api_key = env_api_key;
    }

    normalize_stt_config(config)
}

/// 保存配置
pub fn save_config(config: &SttConfig) -> Result<(), String> {
    let normalized = normalize_stt_config(config.clone());
    let path = paths::config_file();
    paths::ensure_parent(&path)?;
    let json =
        serde_json::to_string_pretty(&normalized).map_err(|e| format!("序列化配置失败: {:?}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("保存配置失败: {:?}", e))
}
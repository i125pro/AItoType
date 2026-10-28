//! 核心业务状态与逻辑（与 GUI 解耦）
//!
//! 这些逻辑可以在**任意线程**调用：GUI 主线程、全局快捷键后台线程、托盘线程。
//! 之所以要这样拆分：主窗口隐藏后，eframe/winit 不再触发 `App::update`，
//! 若把「快捷键录音」「托盘退出」放在 `update` 里轮询就会失效。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audio;
use crate::config;
use crate::corrections;
use crate::keyboard;
use crate::logging;
use crate::settings::UiSettings;
use crate::stt::{self, SttConfig};

lazy_static::lazy_static! {
    pub static ref RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("创建 tokio runtime 失败");
}

pub const HISTORY_LIMIT: usize = 20;

#[derive(PartialEq, Clone, Copy)]
pub enum RecStatus {
    Idle,
    Recording,
    Transcribing,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToastKind {
    Success,
    Error,
}

#[derive(Clone)]
pub struct HistoryItem {
    pub time: String,
    pub text: String,
}

/// 录音 / 转写的共享状态
pub struct Shared {
    pub status: RecStatus,
    pub last_result: String,
    pub show_result: bool,
    pub history: VecDeque<HistoryItem>,
    pub background_session: bool,
    pub hold_started_at: Option<Instant>,
    pub last_toggle_at: Option<Instant>,
    pub toast: Option<(String, ToastKind, Instant)>,
    pub enhancement_hint: Option<(String, Instant)>,
    /// 托盘「显示窗口」请求
    pub show_requested: bool,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            status: RecStatus::Idle,
            last_result: String::new(),
            show_result: false,
            history: VecDeque::new(),
            background_session: false,
            hold_started_at: None,
            last_toggle_at: None,
            toast: None,
            enhancement_hint: None,
            show_requested: false,
        }
    }
}

pub type SharedState = Arc<Mutex<Shared>>;

pub fn new_shared() -> SharedState {
    Arc::new(Mutex::new(Shared::default()))
}

// ============ 提示 / 历史 ============

pub fn set_toast(shared: &SharedState, text: String, kind: ToastKind) {
    if let Ok(mut state) = shared.lock() {
        state.toast = Some((text, kind, Instant::now()));
    }
}

pub fn show_enhancement_hint(shared: &SharedState, reason: String) {
    let detail = reason.trim();
    let truncated = if detail.chars().count() > 120 {
        let prefix: String = detail.chars().take(120).collect();
        format!("{}...", prefix)
    } else {
        detail.to_string()
    };
    let text = if truncated.is_empty() {
        "LLM 润色失败，已回退原始转写".to_string()
    } else {
        format!("LLM 润色失败，已回退原始转写：{}", truncated)
    };
    if let Ok(mut state) = shared.lock() {
        state.enhancement_hint = Some((text, Instant::now()));
    }
}

fn push_history(shared: &SharedState, text: String) {
    if let Ok(mut state) = shared.lock() {
        let time = chrono::Local::now().format("%H:%M").to_string();
        state.history.push_front(HistoryItem { time, text });
        while state.history.len() > HISTORY_LIMIT {
            state.history.pop_back();
        }
    }
}

/// 过期提示清理（由 GUI 每帧调用；后台线程不调用亦可）
pub fn expire_toasts(shared: &SharedState) {
    if let Ok(mut state) = shared.lock() {
        if let Some((_, _, at)) = &state.toast {
            if at.elapsed() > Duration::from_secs(3) {
                state.toast = None;
            }
        }
        if let Some((_, at)) = &state.enhancement_hint {
            if at.elapsed() > Duration::from_secs(5) {
                state.enhancement_hint = None;
            }
        }
    }
}

// ============ 录音流程 ============

pub fn start_recording(shared: &SharedState, background: bool) {
    {
        let state = shared.lock().unwrap();
        if matches!(state.status, RecStatus::Recording | RecStatus::Transcribing) {
            return;
        }
    }

    match audio::start_recording() {
        Ok(()) => {
            let mut state = shared.lock().unwrap();
            state.status = RecStatus::Recording;
            state.background_session = background;
            state.hold_started_at = Some(Instant::now());
            state.show_result = false;
        }
        Err(err) => {
            if let Ok(mut state) = shared.lock() {
                state.background_session = false;
            }
            set_toast(shared, format!("开始录音失败：{}", err), ToastKind::Error);
        }
    }
}

pub fn stop_and_transcribe(shared: &SharedState, config: SttConfig, ui: UiSettings) {
    let background = {
        let mut state = match shared.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        if state.status != RecStatus::Recording {
            return;
        }
        state.status = RecStatus::Transcribing;
        state.background_session
    };

    let auto_copy = ui.auto_copy;
    let auto_write = config.auto_write;
    let shared_handle = shared.clone();

    RUNTIME.spawn(async move {
        let outcome = run_transcribe(config).await;

        match outcome {
            Ok(result) => {
                {
                    let mut state = shared_handle.lock().unwrap();
                    state.status = RecStatus::Idle;
                    state.last_result = result.text.clone();
                    state.show_result = true;
                    state.background_session = false;
                }
                push_history(&shared_handle, result.text.clone());

                if auto_copy {
                    let _ = keyboard::copy_to_clipboard(&result.text);
                }
                if background || auto_write {
                    if let Err(err) = keyboard::paste_text(&result.text) {
                        set_toast(
                            &shared_handle,
                            format!("自动写入失败：{}", err),
                            ToastKind::Error,
                        );
                    }
                }
                if let Some(reason) = result.fallback_reason {
                    show_enhancement_hint(&shared_handle, reason);
                }
            }
            Err(err) => {
                {
                    let mut state = shared_handle.lock().unwrap();
                    state.status = RecStatus::Idle;
                    state.background_session = false;
                }
                set_toast(
                    &shared_handle,
                    format!("转写失败：{}", err),
                    ToastKind::Error,
                );
            }
        }
    });
}

pub fn toggle(shared: &SharedState, config: &SttConfig, ui: &UiSettings, background: bool) {
    let status = match shared.lock() {
        Ok(state) => state.status,
        Err(_) => return,
    };
    match status {
        RecStatus::Recording => stop_and_transcribe(shared, config.clone(), ui.clone()),
        RecStatus::Transcribing => {}
        RecStatus::Idle => start_recording(shared, background),
    }
}

pub fn release_hold(shared: &SharedState, config: &SttConfig, ui: &UiSettings) {
    let elapsed = {
        let state = match shared.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        if state.status != RecStatus::Recording {
            return;
        }
        state.hold_started_at.map(|t| t.elapsed()).unwrap_or_default()
    };

    if elapsed < Duration::from_millis(200) {
        let _ = audio::stop_recording();
        if let Ok(mut state) = shared.lock() {
            state.status = RecStatus::Idle;
            state.background_session = false;
        }
    } else {
        stop_and_transcribe(shared, config.clone(), ui.clone());
    }
}

// ============ 转写 ============

struct TranscribeOutcome {
    text: String,
    fallback_reason: Option<String>,
}

/// 停止录音 -> 转写 -> 纠错 -> 润色 -> 记录日志
async fn run_transcribe(stt_config: SttConfig) -> Result<TranscribeOutcome, String> {
    let file_path = audio::stop_recording()?;
    let config = config::normalize_stt_config(stt_config);

    let transcribe_result = stt::transcribe(&file_path, &config).await;

    if let Err(e) = std::fs::remove_file(&file_path) {
        eprintln!("清理临时录音文件失败 {}: {:?}", file_path, e);
    }

    let raw_text = transcribe_result?;
    let mut store = corrections::load_corrections();

    let first_pass = corrections::apply_corrections(&raw_text, &store);
    let corrected_text = first_pass.text.clone();
    let mut correction_hits = first_pass.hits.clone();
    let mut store_dirty = corrections::increment_hit_counts(&mut store, &first_pass.hits);

    let mut log_entry = logging::TranscribeLogEntry {
        timestamp: chrono::Local::now().to_rfc3339(),
        stt_provider: config.provider.clone(),
        stt_model: config.model.clone(),
        stt_text: raw_text.clone(),
        pre_correction_text: Some(raw_text.clone()),
        post_correction_text: Some(corrected_text.clone()),
        correction_hits: if correction_hits.is_empty() {
            None
        } else {
            Some(correction_hits.clone())
        },
        enhancement_enabled: config.enhancement_enabled,
        enhancement_provider: None,
        enhancement_model: None,
        enhancement_text: None,
        enhancement_status: None,
        enhancement_error: None,
        enhancement_duration_ms: None,
        final_text: corrected_text.clone(),
    };

    if !config.enhancement_enabled {
        if store_dirty {
            let _ = corrections::save_corrections(&store);
        }
        logging::append_log(log_entry);
        return Ok(TranscribeOutcome {
            text: corrected_text,
            fallback_reason: None,
        });
    }

    log_entry.enhancement_provider = Some(config.enhancement_provider.clone());
    log_entry.enhancement_model = Some(config.enhancement_model.clone());

    let enhancement_started = Instant::now();
    let enhance_result = tokio::time::timeout(
        Duration::from_secs(stt::ENHANCEMENT_REQUEST_TIMEOUT_SECS),
        stt::enhance_text(&corrected_text, &config),
    )
    .await;

    let mut fallback_reason: Option<String> = None;
    let after_enhance = match enhance_result {
        Ok(Ok(enhanced_text)) => {
            log_entry.enhancement_text = Some(enhanced_text.clone());
            log_entry.enhancement_status = Some("success".to_string());
            log_entry.enhancement_duration_ms = Some(enhancement_started.elapsed().as_millis() as u64);
            enhanced_text
        }
        Ok(Err(err)) => {
            log_entry.enhancement_status = Some("failed".to_string());
            log_entry.enhancement_error = Some(err.clone());
            log_entry.enhancement_duration_ms = Some(enhancement_started.elapsed().as_millis() as u64);
            fallback_reason = Some(err);
            corrected_text.clone()
        }
        Err(_) => {
            let reason = format!(
                "LLM 润色超时（{} 秒）",
                stt::ENHANCEMENT_REQUEST_TIMEOUT_SECS
            );
            log_entry.enhancement_status = Some("timeout".to_string());
            log_entry.enhancement_error = Some(reason.clone());
            log_entry.enhancement_duration_ms = Some(enhancement_started.elapsed().as_millis() as u64);
            fallback_reason = Some(reason);
            corrected_text.clone()
        }
    };

    let second_pass = corrections::apply_corrections(&after_enhance, &store);
    if !second_pass.hits.is_empty() {
        correction_hits.extend(second_pass.hits.clone());
    }
    store_dirty |= corrections::increment_hit_counts(&mut store, &second_pass.hits);

    log_entry.post_correction_text = Some(second_pass.text.clone());
    log_entry.correction_hits = if correction_hits.is_empty() {
        None
    } else {
        Some(correction_hits)
    };
    log_entry.final_text = second_pass.text.clone();

    if store_dirty {
        let _ = corrections::save_corrections(&store);
    }

    logging::append_log(log_entry);
    Ok(TranscribeOutcome {
        text: second_pass.text,
        fallback_reason,
    })
}
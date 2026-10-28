//! 应用主体：egui 界面 + 业务逻辑编排
//!
//! 这里把原先「Rust 命令 + JS 前端 + 事件总线」的交互，重写为
//! egui 立即模式界面 + 后台线程消息，彻底移除 Tauri / WebView2。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use serde::Deserialize;

use crate::audio;
use crate::config;
use crate::core::{self, HistoryItem, RecStatus, SharedState, ToastKind};
use crate::corrections::{self, CorrectionStore};
use crate::hotkey::HotkeyService;
use crate::keyboard;
use crate::logging;
use crate::settings::{self, UiSettings};
use crate::stt::{self, SttConfig};
#[cfg(not(target_os = "linux"))]
use crate::tray;

const GITHUB_RELEASE_LATEST_API: &str =
    "https://api.github.com/repos/robinshi2010/AItoType/releases/latest";
const DEFAULT_DEVICE_LABEL: &str = "System Default Input";
const WAVEFORM_BARS: usize = 28;
const OVERLAY_WIDTH: f32 = 358.0;
const OVERLAY_HEIGHT: f32 = 110.0;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Record,
    History,
    Settings,
}

struct ConnectionTestResult {
    success: bool,
    provider: String,
    model: String,
    message: String,
    latency_ms: u64,
}

#[derive(Clone)]
struct UpdateInfo {
    latest_version: String,
    release_notes: String,
    release_url: String,
}

enum AsyncMsg {
    Tested(ConnectionTestResult),
    UpdateFound(UpdateInfo),
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
    html_url: String,
}

// ============ 后台任务 ============

fn parse_version_number(version: &str) -> [u32; 3] {
    let cleaned = version.trim().trim_start_matches(['v', 'V']);
    let mut values = [0_u32, 0_u32, 0_u32];
    for (idx, segment) in cleaned.split('.').take(3).enumerate() {
        let digits: String = segment
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        values[idx] = digits.parse::<u32>().unwrap_or(0);
    }
    values
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    parse_version_number(latest) > parse_version_number(current)
}

async fn fetch_update() -> Option<UpdateInfo> {
    tokio::time::sleep(Duration::from_secs(3)).await;

    let current_version = env!("CARGO_PKG_VERSION").to_string();
    if current_version.trim().is_empty() {
        return None;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("AItoType")
        .build()
        .ok()?;

    let response = client
        .get(GITHUB_RELEASE_LATEST_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let release: GitHubRelease = response.json().await.ok()?;
    let latest_version = release
        .tag_name
        .trim()
        .trim_start_matches(['v', 'V'])
        .to_string();
    if latest_version.is_empty() || !is_newer_version(&latest_version, &current_version) {
        return None;
    }

    Some(UpdateInfo {
        latest_version,
        release_notes: release.body.unwrap_or_default(),
        release_url: release.html_url,
    })
}

fn parse_typed_error(input: &str) -> (String, String) {
    if let Some((error_type, message)) = input.split_once('|') {
        return (error_type.to_string(), message.to_string());
    }
    ("unknown".to_string(), input.to_string())
}

async fn run_test_connection(stt_config: SttConfig) -> ConnectionTestResult {
    let provider = stt_config.provider.clone();
    let model = stt_config.model.clone();
    let started_at = Instant::now();

    if !config::has_resolved_api_key(&stt_config) {
        let env_key = stt::env_key_for_provider(&stt_config.provider);
        return ConnectionTestResult {
            success: false,
            provider,
            model,
            message: format!("API Key 不能为空（可通过环境变量 {} 提供）", env_key),
            latency_ms: started_at.elapsed().as_millis() as u64,
        };
    }

    match stt::test_connection(&stt_config).await {
        Ok(_) => ConnectionTestResult {
            success: true,
            provider,
            model,
            message: "连接测试成功".to_string(),
            latency_ms: started_at.elapsed().as_millis() as u64,
        },
        Err(err) => {
            let (_error_type, message) = parse_typed_error(&err);
            ConnectionTestResult {
                success: false,
                provider,
                model,
                message,
                latency_ms: started_at.elapsed().as_millis() as u64,
            }
        }
    }
}

// ============ 权限（macOS） ============

#[cfg(target_os = "macos")]
fn macos_accessibility_trusted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

#[cfg(target_os = "macos")]
fn macos_request_accessibility() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
        fn CFDictionaryCreate(
            allocator: *const std::ffi::c_void,
            keys: *const *const std::ffi::c_void,
            values: *const *const std::ffi::c_void,
            num_values: isize,
            key_callbacks: *const std::ffi::c_void,
            value_callbacks: *const std::ffi::c_void,
        ) -> *const std::ffi::c_void;
        static kAXTrustedCheckOptionPrompt: *const std::ffi::c_void;
        static kCFBooleanTrue: *const std::ffi::c_void;
        static kCFTypeDictionaryKeyCallBacks: *const std::ffi::c_void;
        static kCFTypeDictionaryValueCallBacks: *const std::ffi::c_void;
        fn CFRelease(obj: *const std::ffi::c_void);
    }

    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            kCFTypeDictionaryKeyCallBacks,
            kCFTypeDictionaryValueCallBacks,
        );
        let result = AXIsProcessTrustedWithOptions(options);
        CFRelease(options);
        result
    }
}

fn check_accessibility_permissions() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_accessibility_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn request_accessibility_permissions() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_request_accessibility()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

fn open_external_url(url: &str) {
    let target = url.trim();
    if !(target.starts_with("http://") || target.starts_with("https://")) {
        return;
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(target);
        cmd
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = std::process::Command::new("explorer");
        cmd.arg(target);
        cmd
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(target);
        cmd
    };

    let _ = command.status();
}

// ============ 表单 ============

struct SettingsForm {
    provider: String,
    api_key: String,
    model: String,
    auto_write: bool,
    record_mode_hold: bool,
    enhancement_enabled: bool,
    enhancement_provider: String,
    enhancement_api_key: String,
    enhancement_model: String,
    enhancement_prompt: String,
    corr_wrong: String,
    corr_correct: String,
    capturing: bool,
    capture_hint: String,
    test_result: Option<(bool, String)>,
    testing: bool,
}

impl SettingsForm {
    fn from_config(config: &SttConfig) -> Self {
        Self {
            provider: config.provider.clone(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            auto_write: config.auto_write,
            record_mode_hold: config.record_mode == "hold",
            enhancement_enabled: config.enhancement_enabled,
            enhancement_provider: config.enhancement_provider.clone(),
            enhancement_api_key: config.enhancement_api_key.clone(),
            enhancement_model: config.enhancement_model.clone(),
            enhancement_prompt: config.enhancement_prompt.clone(),
            corr_wrong: String::new(),
            corr_correct: String::new(),
            capturing: false,
            capture_hint: String::new(),
            test_result: None,
            testing: false,
        }
    }
}

fn default_model_for_provider(provider: &str) -> &'static str {
    stt::default_model_for_provider(&stt::normalize_provider(provider))
}

fn default_enhancement_model_for_provider(provider: &str) -> &'static str {
    stt::default_enhancement_model_for_provider(&stt::normalize_provider(provider))
}

// ============ 应用 ============

pub struct AitoTypeApp {
    tab: Tab,

    // ---- 与后台线程共享的业务状态 ----
    shared: SharedState,
    engine_cfg: Arc<Mutex<(SttConfig, UiSettings)>>,
    focused: Arc<AtomicBool>,

    // ---- GUI 本地镜像（每帧从 shared 同步） ----
    status: RecStatus,
    last_result: String,
    show_result: bool,
    history: VecDeque<HistoryItem>,
    toast: Option<(String, ToastKind, Instant)>,
    enhancement_hint: Option<(String, Instant)>,

    stt_config: SttConfig,
    ui_settings: UiSettings,
    form: SettingsForm,
    corrections: CorrectionStore,

    stt_api_keys: HashMap<String, String>,
    enh_api_keys: HashMap<String, String>,

    waveform: Vec<f32>,
    waveform_idx: usize,
    visual_level: f32,
    device_name: String,

    async_tx: Sender<AsyncMsg>,
    async_rx: Receiver<AsyncMsg>,

    hotkey: Option<HotkeyService>,

    _tray: Option<tray_icon::TrayIcon>,
    has_tray: bool,

    window_visible: bool,

    overlay_status: RecStatus,

    update_info: Option<UpdateInfo>,
    update_dismissed: bool,

    accessibility_trusted: bool,
    log_dir: String,

    corr_modal_open: bool,
    corr_modal_wrong: String,
    corr_modal_correct: String,
}

impl AitoTypeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::fonts::install_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let stt_config = config::load_config();
        let ui_settings = settings::load();
        let corrections = corrections::load_corrections();
        let log_dir = logging::get_log_dir_path();

        let mut stt_api_keys = HashMap::new();
        stt_api_keys.insert(stt_config.provider.clone(), stt_config.api_key.clone());
        let mut enh_api_keys = HashMap::new();
        enh_api_keys.insert(
            stt_config.enhancement_provider.clone(),
            stt_config.enhancement_api_key.clone(),
        );

        let mut hotkey = HotkeyService::new().ok();
        if let Some(service) = hotkey.as_mut() {
            if let Err(err) = service.register(&ui_settings.shortcut) {
                eprintln!("注册默认快捷键失败: {}", err);
            }
        }

        let shared = core::new_shared();
        let engine_cfg = Arc::new(Mutex::new((stt_config.clone(), ui_settings.clone())));
        let focused = Arc::new(AtomicBool::new(true));

        // 全局快捷键后台线程：不依赖 GUI 主循环。
        // 这样主窗口隐藏（无重绘、update 不再被调用）时，快捷键依然可用。
        {
            let shared = shared.clone();
            let engine_cfg = engine_cfg.clone();
            let focused = focused.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let receiver = GlobalHotKeyEvent::receiver();
                while let Ok(event) = receiver.recv() {
                    let (cfg, ui) = {
                        let guard = engine_cfg.lock().unwrap();
                        (guard.0.clone(), guard.1.clone())
                    };
                    let background = !focused.load(Ordering::Relaxed);
                    match event.state {
                        HotKeyState::Pressed => {
                            if cfg.record_mode == "hold" {
                                core::start_recording(&shared, background);
                            } else {
                                let now = Instant::now();
                                let skip = match shared.lock() {
                                    Ok(state) => match state.last_toggle_at {
                                        Some(last) => {
                                            now.duration_since(last) < Duration::from_millis(450)
                                        }
                                        None => false,
                                    },
                                    Err(_) => false,
                                };
                                if !skip {
                                    if let Ok(mut state) = shared.lock() {
                                        state.last_toggle_at = Some(now);
                                    }
                                    core::toggle(&shared, &cfg, &ui, background);
                                }
                            }
                        }
                        HotKeyState::Released => {
                            if cfg.record_mode == "hold" {
                                core::release_hold(&shared, &cfg, &ui);
                            }
                        }
                    }
                    // 窗口可见时刷新界面（隐藏时该调用可能无效，但不影响功能）
                    ctx.request_repaint();
                }
            });
        }

        let tray = build_tray_if_supported(shared.clone(), cc.egui_ctx.clone());
        let has_tray = tray.is_some();

        let (async_tx, async_rx) = channel::<AsyncMsg>();
        let update_tx = async_tx.clone();
        core::RUNTIME.spawn(async move {
            if let Some(info) = fetch_update().await {
                let _ = update_tx.send(AsyncMsg::UpdateFound(info));
            }
        });

        let form = SettingsForm::from_config(&stt_config);

        Self {
            tab: Tab::Record,

            shared,
            engine_cfg,
            focused,

            status: RecStatus::Idle,
            last_result: String::new(),
            show_result: false,
            history: VecDeque::new(),
            toast: None,
            enhancement_hint: None,

            stt_config,
            ui_settings,
            form,
            corrections,

            stt_api_keys,
            enh_api_keys,

            waveform: vec![0.0; WAVEFORM_BARS],
            waveform_idx: 0,
            visual_level: 0.0,
            device_name: String::new(),

            async_tx,
            async_rx,

            hotkey,

            _tray: tray,
            has_tray,

            window_visible: true,

            overlay_status: RecStatus::Recording,

            update_info: None,
            update_dismissed: false,

            accessibility_trusted: check_accessibility_permissions(),
            log_dir,

            corr_modal_open: false,
            corr_modal_wrong: String::new(),
            corr_modal_correct: String::new(),
        }
    }

    // ---------- 事件轮询 ----------

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            // 有托盘：隐藏到托盘；无托盘（如 Linux）：直接退出
            if !self.has_tray {
                return;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.window_visible = false;
            self.focused.store(false, Ordering::Relaxed);
        }
    }

    fn handle_show_request(&mut self, ctx: &egui::Context) {
        let requested = match self.shared.lock() {
            Ok(mut state) => {
                if state.show_requested {
                    state.show_requested = false;
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        if requested {
            self.window_visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn poll_async(&mut self) {
        let mut messages = Vec::new();
        while let Ok(msg) = self.async_rx.try_recv() {
            messages.push(msg);
        }

        for msg in messages {
            match msg {
                AsyncMsg::Tested(result) => {
                    self.form.testing = false;
                    let details = format!(
                        "{} / {} / {}ms",
                        format_provider_name(&result.provider),
                        result.model,
                        result.latency_ms
                    );
                    let text = if result.success {
                        format!("✅ {} ({})", result.message, details)
                    } else {
                        format!("❌ {} ({})", result.message, details)
                    };
                    self.form.test_result = Some((result.success, text));
                }
                AsyncMsg::UpdateFound(info) => {
                    if !self.update_dismissed
                        && self.ui_settings.skipped_version != info.latest_version
                    {
                        self.update_info = Some(info);
                    }
                }
            }
        }
    }

    fn poll_recording(&mut self) {
        if self.status != RecStatus::Recording {
            return;
        }

        let level = audio::get_audio_level();
        let normalized = normalize_wave_level(level);
        let smoothing = if normalized > self.visual_level {
            0.36
        } else {
            0.14
        };
        self.visual_level += (normalized - self.visual_level) * smoothing;

        let idx = self.waveform_idx;
        if let Some(slot) = self.waveform.get_mut(idx) {
            *slot = self.visual_level;
        }
        self.waveform_idx = (idx + 1) % self.waveform.len();

        if self.device_name.is_empty() {
            self.device_name = audio::get_input_device_name();
        }
    }

    // ---------- 录音流程（委托给 core，可在后台线程安全调用） ----------

    fn toggle_recording(&self, background: bool) {
        let cfg = self.stt_config.clone();
        let ui = self.ui_settings.clone();
        core::toggle(&self.shared, &cfg, &ui, background);
    }

    // ---------- 提示 ----------

    fn set_toast(&self, text: String, kind: ToastKind) {
        core::set_toast(&self.shared, text, kind);
    }

    fn show_enhancement_hint(&self, reason: String) {
        core::show_enhancement_hint(&self.shared, reason);
    }

    fn expire_toasts(&self) {
        core::expire_toasts(&self.shared);
    }

    // ---------- 与共享状态同步 ----------

    fn sync_from_shared(&mut self) {
        if let Ok(state) = self.shared.lock() {
            self.status = state.status;
            self.last_result = state.last_result.clone();
            self.show_result = state.show_result;
            self.history = state.history.clone();
            self.toast = state.toast.clone();
            self.enhancement_hint = state.enhancement_hint.clone();
            self.overlay_status = state.status;
        }
    }

    fn sync_engine_cfg(&self) {
        if let Ok(mut guard) = self.engine_cfg.lock() {
            *guard = (self.stt_config.clone(), self.ui_settings.clone());
        }
    }

    fn overlay_visible(&self) -> bool {
        match self.shared.lock() {
            Ok(state) => {
                state.background_session
                    && matches!(state.status, RecStatus::Recording | RecStatus::Transcribing)
            }
            Err(_) => false,
        }
    }

    fn set_shared_result(&self, text: String) {
        if let Ok(mut state) = self.shared.lock() {
            state.last_result = text;
        }
    }

    fn set_shared_show_result(&self, show: bool) {
        if let Ok(mut state) = self.shared.lock() {
            state.show_result = show;
        }
    }

    // ---------- 设置保存 ----------

    fn build_config_from_form(&mut self) -> SttConfig {
        let provider = stt::normalize_provider(&self.form.provider);
        let enhancement_provider = stt::normalize_provider(&self.form.enhancement_provider);

        self.stt_api_keys
            .insert(provider.clone(), self.form.api_key.clone());
        self.enh_api_keys
            .insert(enhancement_provider.clone(), self.form.enhancement_api_key.clone());

        let model = if self.form.model.trim().is_empty() {
            default_model_for_provider(&provider).to_string()
        } else {
            self.form.model.trim().to_string()
        };
        let enhancement_model = if self.form.enhancement_model.trim().is_empty() {
            default_enhancement_model_for_provider(&enhancement_provider).to_string()
        } else {
            self.form.enhancement_model.trim().to_string()
        };

        SttConfig {
            provider: provider.clone(),
            base_url: String::new(),
            api_key: self.form.api_key.trim().to_string(),
            model,
            auto_write: self.form.auto_write,
            record_mode: if self.form.record_mode_hold {
                "hold".to_string()
            } else {
                "toggle".to_string()
            },
            enhancement_enabled: self.form.enhancement_enabled,
            enhancement_provider,
            enhancement_base_url: String::new(),
            enhancement_api_key: self.form.enhancement_api_key.trim().to_string(),
            enhancement_model,
            enhancement_prompt: if self.form.enhancement_prompt.trim().is_empty() {
                SttConfig::default().enhancement_prompt
            } else {
                self.form.enhancement_prompt.trim().to_string()
            },
        }
    }

    fn save_settings(&mut self) {
        let built = self.build_config_from_form();
        let normalized = config::normalize_stt_config(built);

        if let Err(err) = config::save_config(&normalized) {
            self.set_toast(format!("保存失败：{}", err), ToastKind::Error);
            return;
        }

        self.stt_config = normalized;

        if let Err(err) = settings::save(&self.ui_settings) {
            self.set_toast(format!("保存设置失败：{}", err), ToastKind::Error);
        } else {
            self.set_toast("设置已保存".to_string(), ToastKind::Success);
        }

        if self.stt_config.auto_write {
            if !self.accessibility_trusted {
                request_accessibility_permissions();
            }
            self.accessibility_trusted = check_accessibility_permissions();
        }

        self.sync_engine_cfg();
    }

    // ---------- 快捷键录制 ----------

    fn handle_shortcut_capture(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            {
                if key == egui::Key::Escape {
                    self.form.capturing = false;
                    self.form.capture_hint = "已取消快捷键设置".to_string();
                    // 恢复原有快捷键，避免取消后处于未注册状态
                    if let Some(service) = self.hotkey.as_mut() {
                        let previous = self.ui_settings.shortcut.clone();
                        let _ = service.register(&previous);
                    }
                    return;
                }

                let Some(main_key) = egui_key_to_token(key) else {
                    continue; // 仅修饰键，继续等待主键
                };

                let mut tokens: Vec<String> = Vec::new();
                if modifiers.command || modifiers.mac_cmd {
                    tokens.push("Cmd".to_string());
                }
                if modifiers.ctrl {
                    tokens.push("Control".to_string());
                }
                if modifiers.alt {
                    tokens.push("Alt".to_string());
                }
                if modifiers.shift {
                    tokens.push("Shift".to_string());
                }
                tokens.push(main_key);

                let shortcut = tokens.join("+");
                self.form.capturing = false;

                if let Some(service) = self.hotkey.as_mut() {
                    match service.register(&shortcut) {
                        Ok(()) => {
                            self.ui_settings.shortcut = shortcut.clone();
                            let _ = settings::save(&self.ui_settings);
                            self.form.capture_hint = format!("快捷键已更新为 {}", shortcut);
                        }
                        Err(err) => {
                            // 恢复上一个可用快捷键
                            let previous = self.ui_settings.shortcut.clone();
                            let _ = service.register(&previous);
                            self.form.capture_hint = format!("设置失败：{}", err);
                        }
                    }
                } else {
                    self.form.capture_hint = "全局快捷键不可用".to_string();
                }
                return;
            }
        }
    }

    // ---------- 上下文菜单 / 关闭 ----------

    fn accessible_label(&self) -> String {
        if self.accessibility_trusted {
            "辅助功能授权正常".to_string()
        } else {
            "需要辅助功能授权才能自动写入".to_string()
        }
    }

    // ---------- 界面 ----------

    fn ui_main(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("aitotype-nav").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("AItoType");
                ui.separator();
                if ui
                    .selectable_label(self.tab == Tab::Record, "录音")
                    .clicked()
                {
                    self.tab = Tab::Record;
                }
                if ui
                    .selectable_label(self.tab == Tab::History, "历史")
                    .clicked()
                {
                    self.tab = Tab::History;
                }
                if ui
                    .selectable_label(self.tab == Tab::Settings, "设置")
                    .clicked()
                {
                    self.tab = Tab::Settings;
                    self.accessibility_trusted = check_accessibility_permissions();
                }

                let header_status = self.status;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), move |ui| {
                    let (text, color) = match header_status {
                        RecStatus::Recording => ("录音中", egui::Color32::from_rgb(0x30, 0xD1, 0x58)),
                        RecStatus::Transcribing => {
                            ("识别中", egui::Color32::from_rgb(0x0A, 0x84, 0xFF))
                        }
                        RecStatus::Idle => ("就绪", egui::Color32::GRAY),
                    };
                    ui.colored_label(color, text);
                });
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Record => self.ui_record(ui),
            Tab::History => self.ui_history(ui),
            Tab::Settings => self.ui_settings(ui),
        });

        self.ui_toast(ctx);
        self.ui_correction_modal(ctx);
    }

    fn ui_record(&mut self, ui: &mut egui::Ui) {
        if let Some(info) = self.update_info.clone() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("发现新版本 v{}", info.latest_version));
                    if ui.button("查看更新").clicked() {
                        open_external_url(&info.release_url);
                    }
                    if ui.button("跳过").clicked() {
                        self.ui_settings.skipped_version = info.latest_version.clone();
                        let _ = settings::save(&self.ui_settings);
                        self.update_info = None;
                    }
                    if ui.button("✕").clicked() {
                        self.update_dismissed = true;
                        self.update_info = None;
                    }
                });
            });
            ui.add_space(8.0);
        }

        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            let (label, color) = match self.status {
                RecStatus::Recording => ("停止录音", egui::Color32::from_rgb(0xFF, 0x45, 0x3A)),
                RecStatus::Transcribing => ("识别中…", egui::Color32::from_rgb(0x0A, 0x84, 0xFF)),
                RecStatus::Idle => ("开始录音", egui::Color32::from_rgb(0x30, 0xD1, 0x58)),
            };

            let size = 132.0 + self.visual_level * 18.0;
            let button = egui::Button::new(
                egui::RichText::new(label).size(20.0).color(egui::Color32::WHITE),
            )
            .min_size(egui::vec2(size, size))
            .fill(color);

            let response = ui.add_enabled(self.status != RecStatus::Transcribing, button);
            if response.clicked() {
                self.toggle_recording(false);
            }

            ui.add_space(10.0);

            if self.status == RecStatus::Recording {
                draw_waveform(ui, &self.waveform, self.waveform_idx, 240.0, 40.0);
                let device = if self.device_name.is_empty() {
                    DEFAULT_DEVICE_LABEL.to_string()
                } else {
                    self.device_name.clone()
                };
                ui.label(format!("🎙 {}", device));
                ui.label(egui::RichText::new("Listening...").weak());
            } else if self.status == RecStatus::Transcribing {
                ui.label(egui::RichText::new("Transcribing...").weak());
            } else {
                let hint = if self.stt_config.record_mode == "hold" {
                    "按住快捷键说话"
                } else {
                    "点击按钮或按快捷键开始"
                };
                ui.label(egui::RichText::new(hint).weak());
            }
        });

        ui.add_space(12.0);

        if self.show_result && !self.last_result.is_empty() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("识别结果").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("关闭").clicked() {
                            self.show_result = false;
                            self.set_shared_show_result(false);
                        }
                        if ui.button("纠错").clicked() {
                            self.corr_modal_open = true;
                            self.corr_modal_wrong = self.last_result.clone();
                            self.corr_modal_correct.clear();
                        }
                        if ui.button("复制").clicked() {
                            let _ = keyboard::copy_to_clipboard(&self.last_result);
                            self.set_toast("已复制".to_string(), ToastKind::Success);
                        }
                    });
                });
                ui.separator();
                let mut text = self.last_result.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
            });
        }

        if let Some((hint, _)) = &self.enhancement_hint {
            ui.add_space(6.0);
            ui.colored_label(egui::Color32::from_rgb(0xFF, 0x9F, 0x0A), hint.as_str());
        }
    }

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        ui.heading("历史记录");
        ui.separator();
        if self.history.is_empty() {
            ui.label(egui::RichText::new("暂无记录").weak());
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            let items: Vec<(usize, String, String)> = self
                .history
                .iter()
                .enumerate()
                .map(|(i, item)| (i, item.time.clone(), item.text.clone()))
                .collect();

            for (i, time, text) in items {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(time).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("复制").clicked() {
                                let _ = keyboard::copy_to_clipboard(&text);
                                self.set_toast("已复制".to_string(), ToastKind::Success);
                            }
                        });
                    });
                    ui.label(text);
                });
                let _ = i;
            }
        });
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("设置");
            ui.separator();

            // 录音模式
            let prev_provider = self.form.provider.clone();
            let prev_enh_provider = self.form.enhancement_provider.clone();

            ui.label(egui::RichText::new("行为").strong());
            ui.checkbox(&mut self.ui_settings.auto_copy, "识别后自动复制到剪贴板");
            ui.checkbox(&mut self.form.auto_write, "识别后自动写入（粘贴到当前光标）");
            ui.checkbox(&mut self.form.record_mode_hold, "按住说话（Push-to-Talk）");

            ui.add_space(4.0);
            if ui.link(self.accessible_label()).clicked() {
                open_accessibility_settings();
            }
            if !self.accessibility_trusted {
                if ui.button("请求辅助功能授权").clicked() {
                    request_accessibility_permissions();
                    self.accessibility_trusted = check_accessibility_permissions();
                }
            }

            ui.add_space(10.0);
            ui.separator();

            // 快捷键
            ui.label(egui::RichText::new("全局快捷键").strong());
            ui.horizontal(|ui| {
                let text = if self.form.capturing {
                    "请按下 1~3 个按键（Esc 取消）…".to_string()
                } else {
                    self.ui_settings.shortcut.clone()
                };
                if ui.button(text).clicked() && !self.form.capturing {
                    self.form.capturing = true;
                    self.form.capture_hint.clear();
                    // 录制期间先注销，避免占用
                    if let Some(service) = self.hotkey.as_mut() {
                        service.unregister();
                    }
                }
                if ui.button("恢复默认").clicked() {
                    let default = UiSettings::default().shortcut;
                    self.ui_settings.shortcut = default.clone();
                    if let Some(service) = self.hotkey.as_mut() {
                        match service.register(&default) {
                            Ok(()) => {
                                let _ = settings::save(&self.ui_settings);
                                self.form.capture_hint = format!("已恢复为 {}", default);
                            }
                            Err(err) => self.form.capture_hint = format!("恢复失败：{}", err),
                        }
                    }
                }
            });
            if !self.form.capture_hint.is_empty() {
                ui.label(egui::RichText::new(self.form.capture_hint.clone()).weak());
            }

            ui.add_space(10.0);
            ui.separator();

            // STT 配置
            ui.label(egui::RichText::new("语音识别（STT）").strong());
            ui.horizontal(|ui| {
                ui.label("Provider");
                egui::ComboBox::from_id_salt("provider-combo")
                    .selected_text(format_provider_name(&self.form.provider))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.form.provider,
                            "openrouter".to_string(),
                            "OpenRouter",
                        );
                        ui.selectable_value(
                            &mut self.form.provider,
                            "siliconflow".to_string(),
                            "SiliconFlow",
                        );
                    });
            });
            if self.form.provider != prev_provider {
                self.stt_api_keys
                    .insert(prev_provider.clone(), self.form.api_key.clone());
                self.form.api_key = self
                    .stt_api_keys
                    .get(&self.form.provider)
                    .cloned()
                    .unwrap_or_default();
                let was_default = self.form.model == default_model_for_provider(&prev_provider)
                    || self.form.model.trim().is_empty();
                if was_default {
                    self.form.model = default_model_for_provider(&self.form.provider).to_string();
                }
            }

            let key_label = if self.form.provider == "siliconflow" {
                "SiliconFlow API Key"
            } else {
                "OpenRouter API Key"
            };
            ui.label(key_label);
            ui.add(
                egui::TextEdit::singleline(&mut self.form.api_key)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );

            ui.label("Model");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.model).desired_width(f32::INFINITY),
            );

            ui.add_space(10.0);
            ui.separator();

            // LLM 润色
            ui.checkbox(&mut self.form.enhancement_enabled, "启用 LLM 润色");
            if self.form.enhancement_enabled {
                ui.horizontal(|ui| {
                    ui.label("Enhancement Provider");
                    egui::ComboBox::from_id_salt("enhancement-provider-combo")
                        .selected_text(format_provider_name(&self.form.enhancement_provider))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.form.enhancement_provider,
                                "openrouter".to_string(),
                                "OpenRouter",
                            );
                            ui.selectable_value(
                                &mut self.form.enhancement_provider,
                                "siliconflow".to_string(),
                                "SiliconFlow",
                            );
                        });
                });
                if self.form.enhancement_provider != prev_enh_provider {
                    self.enh_api_keys.insert(
                        prev_enh_provider.clone(),
                        self.form.enhancement_api_key.clone(),
                    );
                    self.form.enhancement_api_key = self
                        .enh_api_keys
                        .get(&self.form.enhancement_provider)
                        .cloned()
                        .unwrap_or_default();
                    let was_default = self.form.enhancement_model
                        == default_enhancement_model_for_provider(&prev_enh_provider)
                        || self.form.enhancement_model.trim().is_empty();
                    if was_default {
                        self.form.enhancement_model =
                            default_enhancement_model_for_provider(&self.form.enhancement_provider)
                                .to_string();
                    }
                }

                ui.label("Enhancement API Key（留空则复用 STT Key）");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.enhancement_api_key)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );

                ui.label("Enhancement Model");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.enhancement_model)
                        .desired_width(f32::INFINITY),
                );

                ui.label("Enhancement Prompt（{text} 占位符）");
                ui.add(
                    egui::TextEdit::multiline(&mut self.form.enhancement_prompt)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
            }

            ui.add_space(10.0);
            ui.separator();

            // 易错词
            ui.label(egui::RichText::new(format!(
                "易错词记忆（{} 条）",
                self.corrections.corrections.len()
            ))
            .strong());

            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.corr_wrong)
                        .hint_text("错误词")
                        .desired_width(130.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.corr_correct)
                        .hint_text("正确词")
                        .desired_width(130.0),
                );
                if ui.button("添加").clicked() {
                    self.add_correction_from_form();
                }
            });

            let entries: Vec<(String, Vec<String>)> = self
                .corrections
                .corrections
                .iter()
                .map(|entry| (entry.correct.clone(), entry.variants.clone()))
                .collect();

            if entries.is_empty() {
                ui.label(egui::RichText::new("暂无易错词记录").weak());
            } else {
                for (correct, variants) in entries {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(correct.clone()).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("删除整条").clicked() {
                                        self.remove_correction_entry(&correct);
                                    }
                                },
                            );
                        });
                        ui.horizontal_wrapped(|ui| {
                            for variant in &variants {
                                if ui.button(format!("{} ×", variant)).clicked() {
                                    self.remove_correction_variant(&correct, variant);
                                }
                            }
                        });
                    });
                }
            }

            ui.add_space(10.0);
            ui.separator();

            // 连接测试 + 日志
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.form.testing, egui::Button::new("测试连接"))
                    .clicked()
                {
                    self.test_connection();
                }
            });
            if let Some((ok, text)) = &self.form.test_result {
                let color = if *ok {
                    egui::Color32::from_rgb(0x30, 0xD1, 0x58)
                } else {
                    egui::Color32::from_rgb(0xFF, 0x45, 0x3A)
                };
                ui.colored_label(color, text.as_str());
            }
            if self.form.testing {
                ui.label(egui::RichText::new("正在测试连接，请稍候…").weak());
            }

            ui.horizontal(|ui| {
                ui.label("日志目录：");
                ui.label(egui::RichText::new(self.log_dir.clone()).weak());
                if ui.button("打开").clicked() {
                    if let Err(err) = logging::open_log_dir() {
                        self.set_toast(err, ToastKind::Error);
                    }
                }
            });

            ui.add_space(16.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("保存设置").clicked() {
                    self.save_settings();
                }
            });
            ui.add_space(20.0);
        });
    }

    fn ui_toast(&mut self, ctx: &egui::Context) {
        if let Some((text, kind, _)) = self.toast.clone() {
            let color = match kind {
                ToastKind::Success => egui::Color32::from_rgb(0x30, 0xD1, 0x58),
                ToastKind::Error => egui::Color32::from_rgb(0xFF, 0x45, 0x3A),
            };
            egui::Window::new("toast")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -20.0))
                .show(ctx, |ui| {
                    ui.colored_label(color, text);
                });
        }
    }

    fn ui_correction_modal(&mut self, ctx: &egui::Context) {
        if !self.corr_modal_open {
            return;
        }
        let mut open = true;
        egui::Window::new("纠正易错词")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("错误词");
                ui.add(
                    egui::TextEdit::singleline(&mut self.corr_modal_wrong)
                        .desired_width(280.0),
                );
                ui.label("正确词");
                ui.add(
                    egui::TextEdit::singleline(&mut self.corr_modal_correct)
                        .desired_width(280.0),
                );
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        self.corr_modal_open = false;
                    }
                    if ui.button("确认并替换").clicked() {
                        self.confirm_correction_modal();
                    }
                });
            });
        if !open {
            self.corr_modal_open = false;
        }
    }

    // ---------- 纠错操作 ----------

    fn add_correction_from_form(&mut self) {
        let wrong = self.form.corr_wrong.trim().to_string();
        let correct = self.form.corr_correct.trim().to_string();
        if wrong.is_empty() || correct.is_empty() {
            self.set_toast("请输入错误词和正确词".to_string(), ToastKind::Error);
            return;
        }
        if let Err(err) = corrections::add_correction(&mut self.corrections, &wrong, &correct) {
            self.set_toast(err, ToastKind::Error);
            return;
        }
        if let Err(err) = corrections::save_corrections(&self.corrections) {
            self.set_toast(err, ToastKind::Error);
            return;
        }
        self.form.corr_wrong.clear();
        self.form.corr_correct.clear();
        self.set_toast("纠错词已添加".to_string(), ToastKind::Success);
    }

    fn confirm_correction_modal(&mut self) {
        let wrong = self.corr_modal_wrong.trim().to_string();
        let correct = self.corr_modal_correct.trim().to_string();
        if wrong.is_empty() || correct.is_empty() {
            self.set_toast("请输入错误词和正确词".to_string(), ToastKind::Error);
            return;
        }
        if let Err(err) = corrections::add_correction(&mut self.corrections, &wrong, &correct) {
            self.set_toast(err, ToastKind::Error);
            return;
        }
        if let Err(err) = corrections::save_corrections(&self.corrections) {
            self.set_toast(err, ToastKind::Error);
            return;
        }

        if !self.last_result.is_empty() {
            let preview = corrections::apply_corrections(&self.last_result, &self.corrections);
            self.last_result = preview.text;
            self.set_shared_result(self.last_result.clone());
            if self.ui_settings.auto_copy {
                let _ = keyboard::copy_to_clipboard(&self.last_result);
            }
        }

        self.corr_modal_open = false;
        self.set_toast("已保存纠错并更新当前结果".to_string(), ToastKind::Success);
    }

    fn remove_correction_entry(&mut self, correct: &str) {
        corrections::remove_correction(&mut self.corrections, correct);
        let _ = corrections::save_corrections(&self.corrections);
        self.set_toast("已删除纠错条目".to_string(), ToastKind::Success);
    }

    fn remove_correction_variant(&mut self, correct: &str, variant: &str) {
        corrections::remove_correction_variant(&mut self.corrections, correct, variant);
        let _ = corrections::save_corrections(&self.corrections);
        self.set_toast("已删除变体".to_string(), ToastKind::Success);
    }

    // ---------- 异步动作 ----------

    fn test_connection(&mut self) {
        let config = self.build_config_from_form();
        self.form.testing = true;
        self.form.test_result = None;
        let tx = self.async_tx.clone();
        core::RUNTIME.spawn(async move {
            let _ = tx.send(AsyncMsg::Tested(run_test_connection(config).await));
        });
    }

    // ---------- 悬浮条 ----------

    fn ui_overlay(&mut self, ctx: &egui::Context) {
        let viewport_id = egui::ViewportId::from_hash_of("aitotype-overlay");
        let builder = egui::ViewportBuilder::default()
            .with_title("AItoType Overlay")
            .with_inner_size([OVERLAY_WIDTH, OVERLAY_HEIGHT])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false);

        let status = self.overlay_status;
        let level = if status == RecStatus::Recording {
            self.visual_level
        } else {
            0.0
        };
        let device = audio::get_input_device_name();

        ctx.show_viewport_deferred(viewport_id, builder, move |ctx, _class| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(egui::Color32::TRANSPARENT))
                .show(ctx, |ui| {
                    let frame = egui::Frame::none()
                        .fill(egui::Color32::from_rgba_unmultiplied(18, 22, 32, 235))
                        .rounding(egui::Rounding::same(16.0))
                        .inner_margin(egui::Margin::same(12.0));

                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (title, sub) = match status {
                                RecStatus::Transcribing => (
                                    "Transcribing",
                                    if device.is_empty() {
                                        "Processing speech...".to_string()
                                    } else {
                                        format!("{} · Processing...", device)
                                    },
                                ),
                                _ => (
                                    "Recording",
                                    if device.is_empty() {
                                        "Listening...".to_string()
                                    } else {
                                        device.clone()
                                    },
                                ),
                            };
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(title).strong());
                                ui.label(egui::RichText::new(sub).small().weak());
                            });
                            if status == RecStatus::Recording {
                                draw_waveform(ui, &overlay_bars(level), 0, 140.0, 34.0);
                            }
                        });
                    });
                });
        });
    }
}

impl eframe::App for AitoTypeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_from_shared();
        self.sync_engine_cfg();

        self.handle_show_request(ctx);
        self.handle_close_request(ctx);
        self.poll_async();
        self.poll_recording();
        if self.form.capturing {
            self.handle_shortcut_capture(ctx);
        }
        self.expire_toasts();

        if self.window_visible {
            self.ui_main(ctx);
        }
        if self.overlay_visible() {
            self.ui_overlay(ctx);
        }

        self.focused
            .store(ctx.input(|i| i.focused), Ordering::Relaxed);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

// ============ 辅助函数 ============

#[cfg(target_os = "linux")]
fn build_tray_if_supported(
    _shared: SharedState,
    _ctx: egui::Context,
) -> Option<tray_icon::TrayIcon> {
    eprintln!("Linux: 系统托盘暂不启用（需要 GTK 主循环），因此关闭窗口将直接退出程序。");
    None
}

#[cfg(not(target_os = "linux"))]
fn build_tray_if_supported(shared: SharedState, ctx: egui::Context) -> Option<tray_icon::TrayIcon> {
    tray::build_tray(shared, ctx).ok()
}

fn format_provider_name(provider: &str) -> &'static str {
    if stt::normalize_provider(provider) == stt::PROVIDER_SILICONFLOW {
        "SiliconFlow"
    } else {
        "OpenRouter"
    }
}

fn normalize_wave_level(level: f32) -> f32 {
    let safe = if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let boosted = (safe.powf(0.58) * 1.45).min(1.0);
    let threshold = 0.12;
    if boosted <= threshold {
        0.0
    } else {
        ((boosted - threshold) / (1.0 - threshold)).min(1.0)
    }
}

fn egui_key_to_token(key: egui::Key) -> Option<String> {
    use egui::Key;
    let token = match key {
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::Minus => "Minus",
        Key::Equals => "Equal",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        _ => return None,
    };
    Some(token.to_string())
}

/// 录音波形（环形缓冲）
fn draw_waveform(
    ui: &mut egui::Ui,
    bars: &[f32],
    head: usize,
    width: f32,
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    let count = bars.len().max(1);
    let gap = 2.0;
    let bar_width = ((width - gap * (count as f32 - 1.0)) / count as f32).max(2.0);

    for i in 0..count {
        let idx = (head + i) % count;
        let value = bars.get(idx).copied().unwrap_or(0.0);
        let eased = value.powf(0.82);
        let bar_height = if value <= 0.001 {
            2.0
        } else {
            (3.0 + eased * (height - 6.0)).max(3.0)
        };
        let alpha = if value <= 0.001 {
            0.18
        } else {
            0.3 + eased * 0.7
        };
        let x = rect.left() + i as f32 * (bar_width + gap);
        let y = rect.center().y - bar_height / 2.0;
        let bar_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(bar_width, bar_height));
        painter.rect_filled(
            bar_rect,
            egui::Rounding::same(2.0),
            egui::Color32::from_rgba_unmultiplied(60, 170, 255, (alpha * 255.0) as u8),
        );
    }
}

/// 悬浮条使用的伪波形（根据当前电平生成一组对称条）
fn overlay_bars(level: f32) -> Vec<f32> {
    let mut bars = Vec::with_capacity(9);
    for i in 0..9 {
        let center_bias = 1.0 - ((i as f32) - 4.0).abs() / 4.0;
        let envelope = 0.7 + center_bias * 0.55;
        bars.push((level * envelope).clamp(0.0, 1.0));
    }
    bars
}
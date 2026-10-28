//! 全局快捷键服务
//!
//! 基于 `global-hotkey`，替代原 tauri-plugin-global-shortcut。
//! 负责把字符串（如 "Ctrl+Shift+Space"）解析为可注册的 HotKey，
//! 并在应用主循环中轮询按键事件。

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;

/// 快捷键服务，需保持实例存活
pub struct HotkeyService {
    manager: GlobalHotKeyManager,
    current: Option<HotKey>,
}

impl HotkeyService {
    pub fn new() -> Result<Self, String> {
        let manager =
            GlobalHotKeyManager::new().map_err(|e| format!("初始化全局快捷键失败: {}", e))?;
        Ok(Self {
            manager,
            current: None,
        })
    }

    /// 注册（替换）快捷键。传入空字符串表示注销所有。
    pub fn register(&mut self, shortcut: &str) -> Result<(), String> {
        self.unregister();

        let normalized = shortcut.trim();
        if normalized.is_empty() {
            return Ok(());
        }

        let hotkey = parse_shortcut(normalized)?;
        self.manager
            .register(hotkey)
            .map_err(|e| format!("注册快捷键失败: {}", e))?;
        self.current = Some(hotkey);
        Ok(())
    }

    /// 注销当前快捷键
    pub fn unregister(&mut self) {
        if let Some(hotkey) = self.current.take() {
            let _ = self.manager.unregister(hotkey);
        }
    }
}

fn parse_modifier(token: &str) -> Option<Modifiers> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifiers::CONTROL),
        "shift" => Some(Modifiers::SHIFT),
        "alt" | "option" | "opt" => Some(Modifiers::ALT),
        "cmd" | "command" | "super" | "meta" | "win" | "windows" => Some(Modifiers::SUPER),
        _ => None,
    }
}

fn letter_code(ch: char) -> Option<Code> {
    Some(match ch {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        _ => return None,
    })
}

fn digit_code(ch: char) -> Option<Code> {
    Some(match ch {
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        _ => return None,
    })
}

fn function_code(n: u8) -> Option<Code> {
    Some(match n {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        13 => Code::F13,
        14 => Code::F14,
        15 => Code::F15,
        16 => Code::F16,
        17 => Code::F17,
        18 => Code::F18,
        19 => Code::F19,
        20 => Code::F20,
        21 => Code::F21,
        22 => Code::F22,
        23 => Code::F23,
        24 => Code::F24,
        _ => return None,
    })
}

fn parse_code(token: &str) -> Option<Code> {
    let upper = token.to_ascii_uppercase();

    if upper.chars().count() == 1 {
        let ch = upper.chars().next().unwrap();
        if ch.is_ascii_uppercase() {
            return letter_code(ch);
        }
        if ch.is_ascii_digit() {
            return digit_code(ch);
        }
    }

    match upper.as_str() {
        "SPACE" | "SPACEBAR" => Some(Code::Space),
        "ENTER" | "RETURN" => Some(Code::Enter),
        "TAB" => Some(Code::Tab),
        "ESC" | "ESCAPE" => Some(Code::Escape),
        "BACKSPACE" => Some(Code::Backspace),
        "DELETE" | "DEL" => Some(Code::Delete),
        "INSERT" | "INS" => Some(Code::Insert),
        "HOME" => Some(Code::Home),
        "END" => Some(Code::End),
        "PAGEUP" | "PGUP" => Some(Code::PageUp),
        "PAGEDOWN" | "PGDN" => Some(Code::PageDown),
        "UP" | "ARROWUP" => Some(Code::ArrowUp),
        "DOWN" | "ARROWDOWN" => Some(Code::ArrowDown),
        "LEFT" | "ARROWLEFT" => Some(Code::ArrowLeft),
        "RIGHT" | "ARROWRIGHT" => Some(Code::ArrowRight),
        "MINUS" | "-" => Some(Code::Minus),
        "EQUAL" | "PLUS" | "=" | "+" => Some(Code::Equal),
        "COMMA" | "," => Some(Code::Comma),
        "PERIOD" | "." => Some(Code::Period),
        "SLASH" | "/" => Some(Code::Slash),
        "BACKSLASH" | "\\" => Some(Code::Backslash),
        "SEMICOLON" | ";" => Some(Code::Semicolon),
        "QUOTE" | "'" => Some(Code::Quote),
        "BACKQUOTE" | "`" => Some(Code::Backquote),
        "BRACKETLEFT" | "[" => Some(Code::BracketLeft),
        "BRACKETRIGHT" | "]" => Some(Code::BracketRight),
        _ => {
            if upper.len() >= 2 && upper.starts_with('F') {
                if let Ok(n) = upper[1..].parse::<u8>() {
                    return function_code(n);
                }
            }
            None
        }
    }
}

/// 把快捷键字符串解析为 HotKey
pub fn parse_shortcut(input: &str) -> Result<HotKey, String> {
    let mut modifiers = Modifiers::empty();
    let mut code: Option<Code> = None;

    for token in input.split('+') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(modifier) = parse_modifier(token) {
            modifiers |= modifier;
            continue;
        }
        if let Some(parsed) = parse_code(token) {
            code = Some(parsed);
            continue;
        }
        return Err(format!("不支持的按键: {}", token));
    }

    let code = code.ok_or_else(|| "快捷键缺少主按键（例如 A、Space）".to_string())?;
    let modifiers = if modifiers.is_empty() {
        None
    } else {
        Some(modifiers)
    };
    Ok(HotKey::new(modifiers, code))
}
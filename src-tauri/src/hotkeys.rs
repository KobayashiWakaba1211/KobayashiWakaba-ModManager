//! 快捷键：解析 / 修改 mod 目录下 ini 的 [Key*] 段（复刻 QAQM parseIniForHotkeys + save-hotkey）。
//!
//! QAQM 规则：
//! - 段头 `[Key<名>]` 才算快捷键段
//! - 描述名 = 段内最后一个 `$var =` 的变量名（排除 active/active0/active1/pressed/hold），
//!   没有变量则用段名（去掉开头的 `_`）
//! - `key = <表达式>` 行为按键绑定；多行 key 视为备选，展示时首选「纯键盘」的表达式
//! - 保存修改：遍历 mod 目录所有 ini，找到 `[Key<section>]`，把段内 `key =` 行替换为 `key = <新表达式>`

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyEntry {
    /// 段名（Key 后面的部分，与 QAQM save-hotkey 的 sectionName 一致）
    pub section: String,
    /// 描述名（$var 名或段名）
    pub description: String,
    /// 展示按键（已转友好名，如 ["RShift"]、["/"]）
    pub keys: Vec<String>,
    /// 原始 key 表达式（未选中的备选也保留，仅调试/提示用）
    pub raw: String,
}

const EXCLUDED_VARS: [&str; 5] = ["active", "active0", "active1", "pressed", "hold"];

/// 收集 mod 目录下所有 ini（递归，跳过 qaqm 缓存目录）
fn collect_ini_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    let mut files = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name == "qaqm" || name.starts_with("disabled - ") || name.starts_with("do not modify") {
                continue;
            }
            subdirs.push(p);
        } else if p.is_file() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".ini") && !name.ends_with(".ini.bak") {
                files.push(p);
            }
        }
    }
    files.sort();
    out.extend(files);
    subdirs.sort();
    for d in subdirs {
        collect_ini_files(&d, out);
    }
}

/// 把 VK 码 / 记号转成 QAQM 风格的显示名；无法识别的原样保留
fn display_token(tok: &str) -> Option<String> {
    let upper = tok.to_uppercase();
    match upper.as_str() {
        "NO_MODIFIERS" | "NO_SHIFT" | "NO_CTRL" | "NO_ALT" | "NO_WIN" => return None,
        _ => {}
    }
    // 手柄十六进制码（0xCB 之类）：保留原样但加前缀
    if upper.starts_with("0X") {
        return Some(format!("手柄{upper}"));
    }
    let named: &[(&str, &str)] = &[
        ("VK_LBUTTON", "鼠标左键"),
        ("VK_RBUTTON", "鼠标右键"),
        ("VK_MBUTTON", "鼠标中键"),
        ("VK_XBUTTON1", "鼠标侧键1"),
        ("VK_XBUTTON2", "鼠标侧键2"),
        ("VK_SHIFT", "Shift"),
        ("VK_LSHIFT", "LShift"),
        ("VK_RSHIFT", "RShift"),
        ("VK_CONTROL", "Ctrl"),
        ("VK_LCONTROL", "LCtrl"),
        ("VK_RCONTROL", "RCtrl"),
        ("VK_MENU", "Alt"),
        ("VK_LMENU", "LAlt"),
        ("VK_RMENU", "RAlt"),
        ("VK_LWIN", "Win"),
        ("VK_RWIN", "RWin"),
        ("VK_PRIOR", "PageUp"),
        ("VK_NEXT", "PageDown"),
        ("VK_INSERT", "Insert"),
        ("VK_DELETE", "Delete"),
        ("VK_HOME", "Home"),
        ("VK_END", "End"),
        ("VK_UP", "Up"),
        ("VK_DOWN", "Down"),
        ("VK_LEFT", "Left"),
        ("VK_RIGHT", "Right"),
        ("VK_SPACE", "Space"),
        ("VK_TAB", "Tab"),
        ("VK_RETURN", "Enter"),
        ("VK_BACK", "Backspace"),
        ("VK_ESCAPE", "Esc"),
        ("VK_CAPITAL", "CapsLock"),
        ("VK_SNAPSHOT", "PrintScreen"),
        ("VK_SCROLL", "ScrollLock"),
        ("VK_PAUSE", "Pause"),
        ("VK_NUMLOCK", "NumLock"),
        ("VK_MULTIPLY", "Num*"),
        ("VK_ADD", "Num+"),
        ("VK_SUBTRACT", "Num-"),
        ("VK_DECIMAL", "Num."),
        ("VK_DIVIDE", "Num/"),
        ("VK_OEM_1", ";"),
        ("VK_OEM_2", "/"),
        ("VK_OEM_3", "`"),
        ("VK_OEM_4", "["),
        ("VK_OEM_5", "\\"),
        ("VK_OEM_6", "]"),
        ("VK_OEM_7", "'"),
        ("VK_OEM_PLUS", "="),
        ("VK_OEM_MINUS", "-"),
        ("VK_OEM_COMMA", ","),
        ("VK_OEM_PERIOD", "."),
        ("VK_OEM_102", "\\"),
    ];
    for (code, label) in named {
        if upper == *code {
            return Some(label.to_string());
        }
    }
    // VK_NUMPAD0..9
    if let Some(n) = upper.strip_prefix("VK_NUMPAD") {
        if n.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("Num{n}"));
        }
    }
    // VK_F1..VK_F24
    if let Some(n) = upper.strip_prefix("VK_F") {
        if let Ok(k) = n.parse::<u32>() {
            if (1..=24).contains(&k) {
                return Some(format!("F{k}"));
            }
        }
    }
    // VK_0..VK_9 / VK_A..VK_Z
    if let Some(c) = upper.strip_prefix("VK_") {
        if c.len() == 1 && c.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(c.to_string());
        }
    }
    // 单字符原样（"/"、"X" 等）
    if tok.chars().count() == 1 {
        return Some(tok.to_string());
    }
    Some(tok.to_string())
}

/// 判断一个 key 表达式是否「纯键盘」（不含鼠标/手柄码），用于首选展示
fn is_keyboard_only(expr: &str) -> bool {
    expr.split_whitespace().all(|t| {
        let u = t.to_uppercase();
        !u.starts_with("VK_LBUTTON")
            && !u.starts_with("VK_RBUTTON")
            && !u.starts_with("VK_MBUTTON")
            && !u.starts_with("VK_XBUTTON")
            && !u.starts_with("0X")
    })
}

/// 单个表达式的展示按键列表
fn display_keys_for(expr: &str) -> Vec<String> {
    expr.split_whitespace()
        .filter_map(display_token)
        .collect()
}

/// 从 ini 内容解析所有 [Key*] 段的快捷键
fn parse_hotkeys_from_content(content: &str, out: &mut Vec<HotkeyEntry>) {
    let mut current_section: Option<String> = None;
    let mut key_exprs: Vec<String> = Vec::new();
    let mut description: Option<String> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        // 段头
        if line.starts_with('[') && line.ends_with(']') {
            // flush 上一段
            if let Some(sec) = current_section.take() {
                flush_section(&sec, &key_exprs, &description, out);
            }
            key_exprs.clear();
            description = None;
            let inner = &line[1..line.len() - 1];
            if let Some(rest) = inner.strip_prefix("Key").or_else(|| inner.strip_prefix("key")) {
                // 排除大小写歧义：必须原本以 Key 开头（QAQM 用 /i，这里宽松处理即可）
                current_section = Some(rest.to_string());
                // 默认描述：段名去掉开头 _
                let nice = rest.strip_prefix('_').unwrap_or(rest);
                description = Some(nice.to_string());
            }
            continue;
        }
        if current_section.is_none() {
            continue;
        }
        // key = 表达式（大小写不敏感前缀）
        let lower = line.to_lowercase();
        if lower.starts_with("key") {
            if let Some(eq) = line.find('=') {
                let before_eq = line[..eq].trim();
                if before_eq.to_lowercase() == "key" {
                    let expr = line[eq + 1..].trim();
                    if !expr.is_empty() {
                        key_exprs.push(expr.to_string());
                    }
                    continue;
                }
            }
        }
        // $var = （最后一个非排除变量作为描述，与 QAQM 一致）
        if let Some(rest) = line.strip_prefix('$') {
            if let Some(eq) = rest.find('=') {
                let name = rest[..eq].trim();
                if !name.is_empty()
                    && !name.contains('\\') // 命名空间变量 $\QAQM\... 不是显示名
                    && !EXCLUDED_VARS.contains(&name.to_lowercase().as_str())
                {
                    description = Some(name.to_string());
                }
            }
        }
    }
    if let Some(sec) = current_section.take() {
        flush_section(&sec, &key_exprs, &description, out);
    }
}

fn flush_section(
    section: &str,
    key_exprs: &[String],
    description: &Option<String>,
    out: &mut Vec<HotkeyEntry>,
) {
    if key_exprs.is_empty() {
        return;
    }
    // 首选：第一个纯键盘表达式；否则第一个
    let preferred = key_exprs
        .iter()
        .find(|e| is_keyboard_only(e))
        .unwrap_or(&key_exprs[0]);
    let mut keys = display_keys_for(preferred);
    keys.dedup();
    out.push(HotkeyEntry {
        section: section.to_string(),
        description: description.clone().unwrap_or_else(|| section.to_string()),
        keys,
        raw: preferred.clone(),
    });
}

/// 解析 mod 目录下所有 ini 的快捷键
pub fn get_mod_hotkeys(mod_dir: &Path) -> Vec<HotkeyEntry> {
    let mut files = Vec::new();
    collect_ini_files(mod_dir, &mut files);
    let mut out = Vec::new();
    for f in files {
        if let Ok(content) = fs::read_to_string(&f) {
            parse_hotkeys_from_content(&content, &mut out);
        }
    }
    out
}

/// 修改某个 [Key<section>] 段的按键绑定（复刻 QAQM save-hotkey）：
/// 段内所有 `key =` 行合并替换为一行 `key = <新表达式>`（避免重复绑定）
pub fn set_mod_hotkey(
    mod_dir: &Path,
    section: &str,
    new_key: &str,
) -> Result<(), String> {
    let new_key = new_key.trim();
    if new_key.is_empty() || new_key.contains(['\n', '\r']) {
        return Err("按键表达式无效".into());
    }
    let mut files = Vec::new();
    collect_ini_files(mod_dir, &mut files);
    for f in files {
        let Ok(content) = fs::read_to_string(&f) else { continue };
        let lines: Vec<&str> = content.split("\r\n").collect();
        let mut new_lines: Vec<String> = Vec::with_capacity(lines.len());
        let mut in_target = false;
        let mut key_updated = false;
        let mut updated = false;
        for line in &lines {
            let t = line.trim();
            let is_section = t.starts_with('[') && t.ends_with(']')
                && {
                    let inner = &t[1..t.len() - 1];
                    inner.len() > 3
                        && inner[..3].eq_ignore_ascii_case("Key")
                        && &inner[3..] == section
                };
            if is_section {
                // 进入新段：若上一目标段还没写过 key，补一行
                if in_target && !key_updated {
                    new_lines.push(format!("key = {new_key}"));
                    key_updated = true;
                    updated = true;
                }
                in_target = true;
                new_lines.push(line.to_string());
                continue;
            }
            if in_target && t.to_lowercase().starts_with("key") && t.contains('=') {
                // 第一处替换，其余的 key 行丢弃（合并成一行）
                if !key_updated {
                    new_lines.push(format!("key = {new_key}"));
                    key_updated = true;
                    updated = true;
                }
                continue;
            }
            new_lines.push(line.to_string());
        }
        if in_target && !key_updated {
            new_lines.push(format!("key = {new_key}"));
            updated = true;
        }
        if updated {
            let out = new_lines.join("\r\n");
            fs::write(&f, out).map_err(|e| format!("写入失败 {}: {e}", f.display()))?;
            return Ok(());
        }
    }
    Err(format!("未在任何 ini 中找到 [Key{section}] 段"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hotkeys() {
        let content = "[KeyShowTexDebug]\r\nkey = L\r\ntype = hold\r\n$debugTexShow = 1\r\npost $debugTexShow = 0\r\n\r\n\
                      [KeyInputModeToggle]\r\nkey = VK_RSHIFT\r\ntype = cycle\r\n$\\QAQM\\Persist\\Bridge_x\\inputMode = 0, 1\r\n\r\n\
                      [KeyInputManager]\r\nkey = VK_LBUTTON\r\nkey = VK_RBUTTON\r\nkey = X\r\ntype = hold\r\n$isMouseButtonDown = 1\r\n\r\n\
                      [KeyMenu]\r\ncondition = $active == 1\r\nkey = no_modifiers /\r\ntype = cycle\r\n$\\QAQM\\Persist\\Bridge_x\\menu = 0,1\r\n\r\n\
                      [Key_OTH]\r\n$active = 1\r\n$pressed = 1\r\n";
        let mut out = Vec::new();
        parse_hotkeys_from_content(content, &mut out);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].section, "ShowTexDebug");
        assert_eq!(out[0].description, "debugTexShow");
        assert_eq!(out[0].keys, vec!["L"]);
        assert_eq!(out[1].keys, vec!["RShift"]);
        // 鼠标的两个 key 被跳过，首选键盘 X
        assert_eq!(out[2].keys, vec!["X"]);
        assert_eq!(out[2].description, "isMouseButtonDown");
        assert_eq!(out[3].keys, vec!["/"]);
        // 排除变量：OTH 段只有 active/pressed，回退段名（去 _）
        assert_eq!(out[3].section, "Menu");
    }

    #[test]
    fn test_set_hotkey() {
        let tmp = std::env::temp_dir().join(format!("efmi_hotkey_test_{}", std::process::id()));
        let mod_dir = tmp.join("mod");
        fs::create_dir_all(&mod_dir).unwrap();
        let content = "[KeyMenu]\r\nkey = no_modifiers /\r\ntype = cycle\r\n$\\QAQM\\x\\menu = 0,1\r\n";
        fs::write(mod_dir.join("mod.ini"), content).unwrap();

        set_mod_hotkey(&mod_dir, "Menu", "F5").unwrap();
        let after = fs::read_to_string(mod_dir.join("mod.ini")).unwrap();
        assert!(after.contains("key = F5"));
        assert!(!after.contains("no_modifiers"));
        assert!(after.contains("type = cycle"));

        // 段不存在
        assert!(set_mod_hotkey(&mod_dir, "Nope", "F5").is_err());

        fs::remove_dir_all(&tmp).ok();
    }
}

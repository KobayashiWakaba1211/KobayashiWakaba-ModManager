//! 预设快照：保存/恢复「当前启用的 mod 集合」（按游戏隔离）。

use crate::config::{active_game, active_game_mut, load_config, save_config, Preset};
use crate::scan::scan_mods;

/// 获取当前游戏「启用」的 mod 相对路径列表
pub fn current_enabled_mods(mods_path: &str) -> Result<Vec<String>, String> {
    let groups = scan_mods(mods_path)?;
    let mut enabled = Vec::new();
    for g in groups {
        for m in g.mods {
            if m.enabled {
                enabled.push(m.rel_path);
            }
        }
    }
    enabled.sort();
    Ok(enabled)
}

/// 保存当前状态为新预设
pub fn save_preset(mods_path: &str, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("预设名称不能为空".into());
    }
    let enabled = current_enabled_mods(mods_path)?;
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    // 同名则覆盖
    game.presets.retain(|p| p.name != name);
    game.presets.push(Preset {
        name: name.to_string(),
        enabled_mods: enabled,
        created_at: chrono_now(),
    });
    save_config(&cfg)
}

/// 应用某个预设（只启用预设里的 mod，其余禁用）
pub fn apply_preset(mods_path: &str, name: &str) -> Result<(), String> {
    let cfg = load_config();
    let game = active_game(&cfg).ok_or("未配置游戏")?;
    let preset = game
        .presets
        .iter()
        .find(|p| p.name == name)
        .cloned()
        .ok_or_else(|| format!("预设不存在: {name}"))?;

    let target: std::collections::HashSet<String> = preset.enabled_mods.iter().cloned().collect();

    let groups = scan_mods(mods_path)?;
    for g in groups {
        for m in g.mods {
            let should_enable = target.contains(&m.rel_path);
            if m.enabled != should_enable {
                crate::toggle::set_mod_enabled(mods_path, &m.character, &m.name, should_enable)?;
            }
        }
    }
    Ok(())
}

/// 列出当前游戏所有预设
pub fn list_presets() -> Vec<Preset> {
    let cfg = load_config();
    active_game(&cfg).map(|g| g.presets.clone()).unwrap_or_default()
}

/// 删除预设
pub fn delete_preset(name: &str) -> Result<(), String> {
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    let before = game.presets.len();
    game.presets.retain(|p| p.name != name);
    if game.presets.len() == before {
        return Err(format!("预设不存在: {name}"));
    }
    save_config(&cfg)
}

pub(crate) fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let total = secs + 8 * 3600;
    let day = total / 86400;
    let rem = total % 86400;
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    let s = rem % 60;
    let (y, m, d) = days_to_ymd(day as i64);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    let mut y = 1970;
    let mut remaining = days;
    loop {
        let ylen = if is_leap(y) { 366 } else { 365 };
        if remaining < ylen {
            break;
        }
        remaining -= ylen;
        y += 1;
    }
    let mut m = 1;
    let mlen = [31, if is_leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for len in mlen.iter() {
        if remaining < *len {
            break;
        }
        remaining -= *len;
        m += 1;
    }
    (y, m, remaining + 1)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

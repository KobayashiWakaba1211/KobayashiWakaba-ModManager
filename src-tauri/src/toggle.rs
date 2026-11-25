//! Mod 启用/禁用：通过文件夹名加/去 DISABLED_ 前缀实现，并联动 Persist Bridge。

use std::fs;
use std::path::Path;

/// 启用/禁用某个 mod。
/// - mods_path: Mods 根目录
/// - character: 角色名
/// - mod_name: mod 名（干净名，不含 DISABLED_ 前缀）
/// - enabled: true=启用，false=禁用
pub fn set_mod_enabled(mods_path: &str, character: &str, mod_name: &str, enabled: bool) -> Result<(), String> {
    let char_dir = Path::new(mods_path).join(character);
    if !char_dir.is_dir() {
        return Err(format!("角色目录不存在: {character}"));
    }

    let clean = mod_name.trim();
    let current_folder = if enabled {
        // 要启用：当前可能是 DISABLED_xxx，也可能是 xxx
        let disabled = char_dir.join(format!("DISABLED_{clean}"));
        let normal = char_dir.join(clean);
        if disabled.is_dir() {
            disabled
        } else if normal.is_dir() {
            normal
        } else {
            return Err(format!("mod 目录不存在: {character}/{clean}"));
        }
    } else {
        // 要禁用：当前是 xxx
        let normal = char_dir.join(clean);
        if !normal.is_dir() {
            return Err(format!("mod 目录不存在: {character}/{clean}"));
        }
        normal
    };

    let target = if enabled {
        char_dir.join(clean)
    } else {
        char_dir.join(format!("DISABLED_{clean}"))
    };

    // 已是目标状态
    if current_folder == target {
        return Ok(());
    }

    // 目标已存在（同名冲突）
    if target.exists() {
        return Err(format!("目标目录已存在: {}", target.display()));
    }

    fs::rename(&current_folder, &target)
        .map_err(|e| format!("重命名失败: {e}"))?;

    // 联动 Persist Bridge（如果该 mod 被 persist 托管）
    // root = mods_path 的父目录（游戏根目录）
    if let Some(root) = Path::new(mods_path).parent() {
        let _ = crate::persist::sync_mod_toggle(root, &current_folder, enabled);
    }

    Ok(())
}

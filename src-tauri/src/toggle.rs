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
        let _ = crate::persist::sync_mod_toggle(root, &target, enabled);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn persist_scans_ini_and_syncs_after_toggle() -> Result<(), Box<dyn std::error::Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("persist-toggle-{}-{unique}", std::process::id()));
        let mods = root.join("Mods");
        let mod_dir = mods.join("Character").join("Example");
        let nested = mod_dir.join("nested");
        let cache = root.join("qaqm").join("cache");
        fs::create_dir_all(&nested)?;
        fs::create_dir_all(&cache)?;
        let state = "qaqm_state_test.ini";
        fs::write(nested.join("mod.INI"), format!("; Persisted state is hosted by {state}\n"))?;
        fs::write(nested.join("ignored.txt"), "not an ini")?;
        let files = crate::persist::get_ini_files(&mod_dir);
        assert_eq!(files, vec![nested.join("mod.INI")]);
        fs::write(cache.join(state), "namespace = QAQM\\Persist\\Bridge_test\nglobal persist $part = 0\n")?;
        fs::write(root.join("d3dx_user.ini"), "[Constants]\n$\\QAQM\\Persist\\Bridge_test\\part = 7\n")?;
        let mods_path = mods.to_string_lossy();
        set_mod_enabled(&mods_path, "Character", "Example", false)?;
        assert!(mods.join("Character").join("DISABLED_Example").is_dir());
        assert!(fs::read_to_string(cache.join(state))?.contains("global persist $part = 7"));
        let active = root.join("qaqm").join("qaqm_persist_active.ini");
        fs::write(&active, "")?;
        set_mod_enabled(&mods_path, "Character", "Example", true)?;
        assert!(mod_dir.is_dir());
        assert!(fs::read_to_string(&active)?.contains(state));
        fs::remove_file(nested.join("mod.INI"))?;
        fs::remove_file(nested.join("ignored.txt"))?;
        fs::remove_dir(&nested)?;
        fs::remove_dir(&mod_dir)?;
        fs::remove_dir(mods.join("Character"))?;
        fs::remove_dir(&mods)?;
        fs::remove_file(cache.join(state))?;
        fs::remove_dir(&cache)?;
        fs::remove_file(active)?;
        fs::remove_dir(root.join("qaqm"))?;
        fs::remove_file(root.join("d3dx_user.ini"))?;
        fs::remove_dir(root)?;
        Ok(())
    }
}

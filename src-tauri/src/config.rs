//! 配置读写：持久化多游戏 Mods 路径、预设等设置到应用数据目录。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 游戏列表
    pub games: Vec<Game>,
    /// 当前激活的游戏 id
    pub active_game_id: String,
    /// Persist Bridge 是否启用
    pub persist_enabled: bool,
    /// Persist Bridge 缓存：最近保留的 mod 数量
    pub persist_cache_size: usize,
    /// 被用户隐藏的内置游戏 id（不想玩的游戏从列表里隐藏，可恢复）
    #[serde(default)]
    pub hidden_games: Vec<String>,
    /// 角色详情页的「单选模式」开关（持久化，重启后保持）
    #[serde(default)]
    pub single_mode: bool,
    /// 角色一览卡片宽度 px（系统设置滑块可调，120~260）
    #[serde(default)]
    pub card_size: u32,
    /// 画廊视图 Mod 卡片宽度 px（系统设置滑块可调，140~320）
    #[serde(default)]
    pub gallery_card_size: u32,
    /// Mod 市场中央推荐大卡片的显示大小（占容器宽度的百分比，40~100）
    #[serde(default)]
    pub market_hero_scale: u32,
    /// Mod 市场小卡片宽度 px（系统设置滑块可调，150~320）
    #[serde(default)]
    pub market_card_size: u32,
    /// 市场下载文件夹（「仅下载」保存路径；空 = 系统默认下载文件夹）
    #[serde(default)]
    pub download_dir: String,
    /// XXMI Launcher.exe 路径（全局，所有游戏共用；空 = 从各游戏遗留值或默认安装位置推断）
    #[serde(default)]
    pub xxmi_launcher_path: String,
    /// 窗口宽度（px，记住上次关闭时的大小，0 = 未记录，用默认 800）
    #[serde(default)]
    pub window_width: u32,
    /// 窗口高度（px，0 = 未记录，用默认 600）
    #[serde(default)]
    pub window_height: u32,
    /// 悬浮窗全局快捷键
    pub overlay_hotkey: String,
    #[serde(skip)]
    original: Option<serde_json::Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            games: Vec::new(),
            active_game_id: String::new(),
            persist_enabled: false,
            persist_cache_size: 30,
            hidden_games: Vec::new(),
            single_mode: false,
            card_size: 158,
            gallery_card_size: 200,
            market_hero_scale: 100,
            market_card_size: 210,
            download_dir: String::new(),
            xxmi_launcher_path: String::new(),
            window_width: 0,
            window_height: 0,
            overlay_hotkey: "Alt+F".to_string(),
            original: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    /// 唯一 id（用游戏名拼音/英文，如 "endfield"、"starrail"）
    pub id: String,
    /// 显示名（如 "明日方舟终末地"）
    pub name: String,
    /// Mods 根目录
    pub mods_path: String,
    /// 该游戏的预设列表
    #[serde(default)]
    pub presets: Vec<Preset>,
    /// 角色皮肤分类（外观分区），如「原版」「泳装」等自定义分组
    #[serde(default)]
    pub sections: Vec<CharacterSection>,
    /// Mod 归属分区：key = "角色/Mod名"（rel_path），value = 分区 id；缺省表示属于「原角色」
    #[serde(default)]
    pub mod_sections: std::collections::HashMap<String, String>,
    /// Mod 备注：key = rel_path，value = 备注文本
    #[serde(default)]
    pub mod_notes: std::collections::HashMap<String, String>,
    /// 置顶的 Mod（rel_path 列表，排序时置顶优先）
    #[serde(default)]
    pub pinned_mods: Vec<String>,
    /// 角色自定义顺序（角色名列表，排序方式选「自定义」时按此顺序，未列出的角色按名称排后面）
    #[serde(default)]
    pub character_order: Vec<String>,
    /// 市场安装来源：key = rel_path，value = GameBanana 来源信息（用于更新检测）
    #[serde(default)]
    pub market_sources: std::collections::HashMap<String, MarketSourceRecord>,
    /// 用户自定义绑定的 GameBanana 游戏 id（自定义游戏市场兜底；0 = 未绑定）
    #[serde(default)]
    pub gb_game_id: u32,
    /// 用户自定义绑定的 GameBanana 分类根 id（分类列表起点；0 = 用游戏 id 默认）
    #[serde(default)]
    pub gb_category_id: u32,
    /// 游戏本体路径（原版启动时执行的 exe / 官方启动器；空 = 从 XXMI 配置推断）
    #[serde(default)]
    pub game_path: String,
    /// XXMI Launcher.exe 路径（EFMI/XXMI 等注入模式启动用；空 = 用默认安装位置）
    #[serde(default)]
    pub launcher_path: String,
    /// GIML.exe 路径（原神的加壳启动器；空 = 自动在桌面/下载目录里找 GIML.exe）
    #[serde(default)]
    pub giml_path: String,
    /// 启动模式："" / "VANILLA"（原版）、"EFMI"、"XXMI"、"SRMI"、"WWMI"、"ZZMI"、"GIMI"
    #[serde(default)]
    pub launch_mode: String,
    /// 附加启动参数（拼到命令末尾）
    #[serde(default)]
    pub launch_args: String,
}

/// 市场安装来源记录（复刻 IMM 的 update tracking）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSourceRecord {
    /// GameBanana 的 mod id
    pub gb_mod_id: u32,
    /// GameBanana 页面链接
    pub page_url: String,
    /// 安装时间（Unix 毫秒）；检测更新时与 GB 最新文件时间比较
    pub installed_at: i64,
}

/// 角色皮肤分类（外观分区）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterSection {
    /// 唯一 id，形如 custom:<时间戳>
    pub id: String,
    /// 所属角色名
    pub character: String,
    /// 分类显示名
    pub name: String,
    /// 创建时间
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    /// 预设里「启用」的 mod 相对路径列表（相对该游戏 mods_path，去掉 DISABLED_ 前缀后的干净路径）
    pub enabled_mods: Vec<String>,
    pub created_at: String,
}

/// 获取配置文件路径（AppData 下的 endfield-mod-manager 目录）
pub fn config_dir() -> PathBuf {
    let base = if let Some(dir) = std::env::var_os("APPDATA") {
        PathBuf::from(dir)
    } else if let Some(dir) = std::env::var_os("HOME") {
        PathBuf::from(dir).join(".config")
    } else {
        PathBuf::from(".")
    };
    base.join("endfield-mod-manager")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn read_config(path: &Path) -> Result<(AppConfig, serde_json::Value), String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let cfg = AppConfig::default();
            let value = serde_json::to_value(&cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
            return Ok((cfg, value));
        }
        Err(e) => return Err(format!("读取配置失败: {e}")),
    };
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("配置文件损坏，已保留原文件: {e}"))?;
    let cfg = serde_json::from_value(value.clone())
        .map_err(|e| format!("配置格式无效，已保留原文件: {e}"))?;
    Ok((cfg, value))
}

pub fn load_config() -> AppConfig {
    let result = CONFIG_LOCK
        .lock()
        .map_err(|e| format!("配置锁不可用: {e}"))
        .and_then(|_guard| read_config(&config_path()));
    match result {
        Ok((mut cfg, _)) => {
            cfg.original = serde_json::to_value(&cfg).ok();
            cfg
        }
        Err(e) => {
            eprintln!("{e}");
            AppConfig::default()
        }
    }
}

// 只应用调用者实际更改的字段；同一字段并发更改时拒绝覆盖。
fn merge_changes(
    base: &serde_json::Value,
    edited: &serde_json::Value,
    current: &mut serde_json::Value,
) -> Result<(), String> {
    if base == edited {
        return Ok(());
    }
    if let (Some(base_items), Some(edited_items), Some(current_items)) =
        (base.as_array(), edited.as_array(), current.as_array_mut())
    {
        let id = |v: &serde_json::Value| v.get("id").and_then(|v| v.as_str()).map(str::to_owned);
        let old_order: Vec<_> = base_items
            .iter()
            .filter_map(&id)
            .filter(|key| edited_items.iter().any(|v| id(v).as_ref() == Some(key)))
            .collect();
        let new_order: Vec<_> = edited_items
            .iter()
            .filter_map(&id)
            .filter(|key| base_items.iter().any(|v| id(v).as_ref() == Some(key)))
            .collect();
        if old_order == new_order
            && base_items
                .iter()
                .chain(edited_items)
                .chain(current_items.iter())
                .all(|v| id(v).is_some())
        {
            for old in base_items {
                let Some(old_id) = id(old) else { continue };
                let edited_item = edited_items
                    .iter()
                    .find(|v| id(v).as_deref() == Some(&old_id));
                let position = current_items
                    .iter()
                    .position(|v| id(v).as_deref() == Some(&old_id));
                match (edited_item, position) {
                    (Some(new), Some(pos)) => merge_changes(old, new, &mut current_items[pos])?,
                    (None, Some(pos)) if &current_items[pos] == old => {
                        current_items.remove(pos);
                    }
                    (None, None) => {}
                    (Some(new), None) if new == old => {}
                    _ => return Err("配置已被其他操作修改，请重试".into()),
                }
            }
            for new in edited_items {
                let Some(new_id) = id(new) else { continue };
                if !base_items.iter().any(|v| id(v).as_deref() == Some(&new_id)) {
                    match current_items
                        .iter()
                        .find(|v| id(v).as_deref() == Some(&new_id))
                    {
                        Some(existing) if existing != new => {
                            return Err("配置已被其他操作修改，请重试".into());
                        }
                        Some(_) => {}
                        None => current_items.push(new.clone()),
                    }
                }
            }
            return Ok(());
        }
    }
    if let (Some(base), Some(edited), Some(current)) = (
        base.as_object(),
        edited.as_object(),
        current.as_object_mut(),
    ) {
        for (key, old) in base {
            if !edited.contains_key(key) {
                match current.get(key) {
                    Some(value) if value != old => {
                        return Err("配置已被其他操作修改，请重试".into());
                    }
                    Some(_) => {
                        current.remove(key);
                    }
                    None => {}
                }
            }
        }
        for (key, value) in edited {
            match base.get(key) {
                Some(old) if old != value => {
                    let target = current.entry(key.clone()).or_insert_with(|| old.clone());
                    merge_changes(old, value, target)?;
                }
                None => match current.get(key) {
                    Some(existing) if existing != value => {
                        return Err("配置已被其他操作修改，请重试".into());
                    }
                    Some(_) => {}
                    None => {
                        current.insert(key.clone(), value.clone());
                    }
                },
                _ => {}
            }
        }
        return Ok(());
    }
    if current != base && current != edited {
        return Err("配置已被其他操作修改，请重试".into());
    }
    *current = edited.clone();
    Ok(())
}

pub fn save_config(cfg: &AppConfig) -> Result<(), String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|e| format!("配置锁不可用: {e}"))?;
    save_config_at(&config_path(), cfg)
}

fn save_config_at(path: &Path, cfg: &AppConfig) -> Result<(), String> {
    let base = cfg
        .original
        .as_ref()
        .ok_or("配置未成功读取，拒绝覆盖原文件")?;
    let dir = path.parent().ok_or("配置路径无父目录")?;
    fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("config.lock"))
        .map_err(|e| format!("打开配置锁失败: {e}"))?;
    lock.lock().map_err(|e| format!("锁定配置失败: {e}"))?;
    let (current_cfg, mut current_raw) = read_config(path)?;
    let current = serde_json::to_value(&current_cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
    let edited = serde_json::to_value(cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
    let mut merged = current;
    merge_changes(base, &edited, &mut merged)?;
    merge_changes(
        &serde_json::to_value(&current_cfg).map_err(|e| format!("序列化配置失败: {e}"))?,
        &merged,
        &mut current_raw,
    )?;
    let raw =
        serde_json::to_vec_pretty(&current_raw).map_err(|e| format!("序列化配置失败: {e}"))?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(dir).map_err(|e| format!("创建临时配置失败: {e}"))?;
    tmp.write_all(&raw)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(|e| format!("写入临时配置失败: {e}"))?;
    tmp.persist(path)
        .map_err(|e| format!("替换配置文件失败: {}", e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(path: &Path) -> AppConfig {
        let (mut cfg, _) = read_config(path).expect("读取测试配置");
        cfg.original = Some(serde_json::to_value(&cfg).expect("序列化测试配置"));
        cfg
    }

    #[test]
    fn concurrent_snapshots_preserve_independent_changes_and_unknown_fields() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"future_field":{"keep":true}}"#).expect("写入配置");
        let mut first = snapshot(&path);
        let mut second = snapshot(&path);
        first.card_size = 180;
        second.overlay_hotkey = "Ctrl+F".into();
        save_config_at(&path, &first).expect("保存第一个修改");
        save_config_at(&path, &second).expect("保存第二个修改");
        let (result, raw) = read_config(&path).expect("读取结果");
        assert_eq!(result.card_size, 180);
        assert_eq!(result.overlay_hotkey, "Ctrl+F");
        assert_eq!(raw["future_field"]["keep"], true);
    }

    #[test]
    fn conflicting_snapshots_are_rejected() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("config.json");
        let mut first = snapshot(&path);
        let mut second = snapshot(&path);
        first.card_size = 180;
        second.card_size = 200;
        save_config_at(&path, &first).expect("保存第一个修改");
        assert!(save_config_at(&path, &second).is_err());
        assert_eq!(read_config(&path).expect("读取结果").0.card_size, 180);
    }

    #[test]
    fn simultaneous_writes_are_serialized() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("config.json");
        let mut first = snapshot(&path);
        let mut second = snapshot(&path);
        first.card_size = 180;
        second.gallery_card_size = 240;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let other_path = path.clone();
        let other_barrier = barrier.clone();
        let thread = std::thread::spawn(move || {
            other_barrier.wait();
            save_config_at(&other_path, &first)
        });
        barrier.wait();
        save_config_at(&path, &second).expect("保存第二个修改");
        thread
            .join()
            .expect("线程正常结束")
            .expect("保存第一个修改");
        let result = read_config(&path).expect("读取结果").0;
        assert_eq!((result.card_size, result.gallery_card_size), (180, 240));
    }

    #[test]
    fn corrupt_configuration_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("config.json");
        let cfg = snapshot(&path);
        fs::write(&path, "{broken").expect("写入损坏配置");
        assert!(save_config_at(&path, &cfg).is_err());
        assert_eq!(fs::read_to_string(&path).expect("读取原文件"), "{broken");
        assert!(save_config_at(&path, &AppConfig::default()).is_err());
    }
}

/// 判断路径是否是一个 Mods 目录（路径任一层级的名字含 "mods"，大小写不敏感）
pub fn is_mods_path(p: &str) -> bool {
    let t = p.trim();
    !t.is_empty()
        && Path::new(t).components().any(|c| {
            c.as_os_str()
                .to_string_lossy()
                .to_lowercase()
                .contains("mods")
        })
}

/// 校验 mods_path（只要求路径含 mods，不要求目录当前就存在）
pub fn validate_mods_path(p: &str) -> bool {
    is_mods_path(p)
}

/// 获取当前激活游戏
pub fn active_game(cfg: &AppConfig) -> Option<&Game> {
    cfg.games
        .iter()
        .find(|g| g.id == cfg.active_game_id)
        .or_else(|| cfg.games.first())
}

/// 获取当前激活游戏（可变）
pub fn active_game_mut(cfg: &mut AppConfig) -> Option<&mut Game> {
    let id = cfg.active_game_id.clone();
    // 先找匹配 id 的，找不到则用第一个
    let idx = cfg
        .games
        .iter()
        .position(|g| g.id == id)
        .or_else(|| if cfg.games.is_empty() { None } else { Some(0) });
    idx.and_then(move |i| cfg.games.get_mut(i))
}

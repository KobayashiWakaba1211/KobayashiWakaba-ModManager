mod characters;
mod config;
mod gallery;
mod hotkeys;
mod import_mod;
mod instance;
mod launch;
mod market;
mod modpack;
mod persist;
mod preset;
mod scan;
mod toggle;
mod update;

use market::{cancel_market_download, market_categories, market_check_updates, market_comments, market_download, market_download_file, market_mod_detail, market_mods, market_search, market_top_subs};
use tauri::Manager;

use config::{active_game, active_game_mut, load_config, save_config, AppConfig, Game};
use scan::{clean_name, scan_mods, CharacterGroup};

// ===== 内置游戏定义（按 XXMI 安装目录名识别） =====
/// 内置游戏映射：目录名 -> (id, 显示名)
const BUILTIN_GAMES: &[(&str, &str, &str)] = &[
    ("EndField Game", "endfield", "明日方舟终末地"),
    ("Star Rail Game", "starrail", "崩坏：星穹铁道"),
    ("Wuthering Waves", "wuthering", "鸣潮"),
    ("ZenlessZoneZero", "zenless", "绝区零"),
    // 原神：用户手动添加过（id 与已有记录一致，避免重复游戏）
    ("GenshinImpact", "GenshinImpact", "原神"),
];

/// 从目录名推断游戏 id 和显示名
fn infer_game(dir_name: &str) -> Option<(&'static str, &'static str)> {
    BUILTIN_GAMES
        .iter()
        .find(|(dir, _, _)| *dir == dir_name)
        .map(|(_, id, name)| (*id, *name))
}

// ===== Tauri 命令 =====

/// 构造一个「空路径」的内置游戏记录（Mods 路径待用户配置）
fn builtin_game(id: &str, name: &str) -> Game {
    Game {
        id: id.to_string(),
        name: name.to_string(),
        market_sources: std::collections::HashMap::new(),
        mods_path: String::new(),
        presets: Vec::new(),
        sections: Vec::new(),
        mod_sections: std::collections::HashMap::new(),
        mod_notes: std::collections::HashMap::new(),
        pinned_mods: Vec::new(),
        character_order: Vec::new(),
        gb_game_id: 0,
        gb_category_id: 0,
        game_path: String::new(),
        launcher_path: String::new(),
        giml_path: String::new(),
        launch_mode: String::new(),
        launch_args: String::new(),
    }
}

/// 首次启动时预置五款内置游戏（空路径），让干净环境也能直接看到游戏列表，
/// 而不是必须先「添加游戏」选 Mods 目录。仅在 games 完全为空时执行一次。
fn ensure_builtin_games(cfg: &mut AppConfig) -> bool {
    if !cfg.games.is_empty() {
        return false;
    }
    for (_, id, name) in BUILTIN_GAMES {
        cfg.games.push(builtin_game(id, name));
    }
    // 默认激活第一款内置游戏
    if cfg.active_game_id.is_empty() {
        cfg.active_game_id = BUILTIN_GAMES[0].1.to_string();
    }
    let _ = save_config(cfg);
    true
}

/// 列出所有已配置的游戏（首次启动时自动预置内置游戏）
#[tauri::command]
fn list_games() -> Vec<Game> {
    let mut cfg = load_config();
    ensure_builtin_games(&mut cfg);
    cfg.games
}

/// 获取当前激活游戏的 id（首次启动时自动预置内置游戏）
#[tauri::command]
fn get_active_game_id() -> String {
    let mut cfg = load_config();
    ensure_builtin_games(&mut cfg);
    cfg.active_game_id
}

/// 设置当前激活游戏（切换后按其固化角色名单补齐 Mods 目录结构）
#[tauri::command]
fn set_active_game(id: String) -> Result<(), String> {
    let mut cfg = load_config();
    if !cfg.games.iter().any(|g| g.id == id) {
        // 内置游戏尚未添加：直接建立空路径记录，Mods 路径改到「系统设置 → 当前游戏」里配置
        let default_name = BUILTIN_GAMES
            .iter()
            .find(|(_, gid, _)| *gid == id)
            .map(|(_, _, name)| name.to_string())
            .ok_or_else(|| format!("游戏不存在: {id}"))?;
        cfg.games.push(Game {
            id: id.clone(),
            name: default_name,
            market_sources: std::collections::HashMap::new(),
            mods_path: String::new(),
            presets: Vec::new(),
            sections: Vec::new(),
            mod_sections: std::collections::HashMap::new(),
            mod_notes: std::collections::HashMap::new(),
            pinned_mods: Vec::new(),
            character_order: Vec::new(),
            gb_game_id: 0,
            gb_category_id: 0,
            game_path: String::new(),
            launcher_path: String::new(),
            giml_path: String::new(),
            launch_mode: String::new(),
            launch_args: String::new(),
        });
    }
    let mods_path = cfg
        .games
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.mods_path.clone())
        .unwrap_or_default();
    // 路径含 mods 时，检查固化角色文件夹：缺失则创建，已存在则保留
    characters::ensure_character_dirs(&id, &mods_path)?;
    cfg.active_game_id = id;
    save_config(&cfg)
}

/// 添加一个游戏（指定 Mods 目录，自动推断 id/name）
#[tauri::command]
fn add_game(mods_path: String) -> Result<Game, String> {
    if !config::validate_mods_path(&mods_path) {
        return Err(format!("路径不含 mods: {mods_path}"));
    }
    // 从路径推断游戏名：取 Mods 的上级目录名
    let p = std::path::Path::new(&mods_path);
    let dir_name = p
        .parent()
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "自定义游戏".to_string());

    let (id, name) = match infer_game(&dir_name) {
        Some((id, name)) => (id.to_string(), name.to_string()),
        None => {
            // 自定义：用目录名做 id 和 name
            let id = dir_name.clone();
            let name = dir_name.clone();
            (id, name)
        }
    };

    // 路径含 mods 时，按其固化角色名单补齐目录结构
    characters::ensure_character_dirs(&id, &mods_path)?;

    let mut cfg = load_config();
    // 已存在同 mods_path 则直接返回
    if let Some(existing) = cfg.games.iter().find(|g| g.mods_path == mods_path) {
        return Ok(existing.clone());
    }
    let game = Game {
        id: id.clone(),
        name: name.clone(),
        mods_path,
        market_sources: std::collections::HashMap::new(),
        presets: Vec::new(),
        sections: Vec::new(),
        mod_sections: std::collections::HashMap::new(),
        mod_notes: std::collections::HashMap::new(),
        pinned_mods: Vec::new(),
        character_order: Vec::new(),
        gb_game_id: 0,
        gb_category_id: 0,
        game_path: String::new(),
        launcher_path: String::new(),
        giml_path: String::new(),
        launch_mode: String::new(),
        launch_args: String::new(),
    };
    cfg.games.push(game.clone());
    if cfg.active_game_id.is_empty() {
        cfg.active_game_id = game.id.clone();
    }
    save_config(&cfg)?;
    Ok(game)
}

/// 移除游戏
#[tauri::command]
fn remove_game(id: String) -> Result<(), String> {
    let mut cfg = load_config();
    let before = cfg.games.len();
    cfg.games.retain(|g| g.id != id);
    if cfg.games.len() == before {
        return Err(format!("游戏不存在: {id}"));
    }
    if cfg.active_game_id == id {
        cfg.active_game_id = cfg.games.first().map(|g| g.id.clone()).unwrap_or_default();
    }
    save_config(&cfg)
}

/// 获取当前激活游戏的 mods_path（空字符串表示未配置）
#[tauri::command]
fn get_mods_path() -> String {
    let cfg = load_config();
    active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default()
}

/// 设置当前激活游戏的 mods_path
#[tauri::command]
fn set_mods_path(path: String) -> Result<(), String> {
    if !config::validate_mods_path(&path) {
        return Err(format!("路径不含 mods: {path}"));
    }
    let mut cfg = load_config();
    let game = config::active_game_mut(&mut cfg).ok_or("尚未添加游戏")?;
    let id = game.id.clone();
    // 路径含 mods 时，按其固化角色名单补齐目录结构
    characters::ensure_character_dirs(&id, &path)?;
    game.mods_path = path;
    save_config(&cfg)
}

#[tauri::command]
fn scan() -> Result<Vec<CharacterGroup>, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    scan_mods(&path)
}

#[tauri::command]
fn get_character_gallery(character: String) -> Result<Vec<gallery::GalleryMod>, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    gallery::collect_gallery(&path, &character)
}

#[tauri::command]
fn get_single_mode() -> bool {
    load_config().single_mode
}

#[tauri::command]
fn set_single_mode(enabled: bool) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.single_mode = enabled;
    save_config(&cfg)
}

#[tauri::command]
fn get_card_size() -> u32 {
    let n = load_config().card_size;
    if (120..=260).contains(&n) { n } else { 158 }
}

#[tauri::command]
fn set_card_size(size: u32) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.card_size = size.clamp(120, 260);
    save_config(&cfg)
}

#[tauri::command]
fn get_gallery_card_size() -> u32 {
    let n = load_config().gallery_card_size;
    if (140..=320).contains(&n) { n } else { 200 }
}

#[tauri::command]
fn set_gallery_card_size(size: u32) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.gallery_card_size = size.clamp(140, 320);
    save_config(&cfg)
}

/// 市场中央推荐大卡片的显示大小（占容器宽度的百分比）
#[tauri::command]
fn get_market_hero_scale() -> u32 {
    let n = load_config().market_hero_scale;
    if (40..=100).contains(&n) { n } else { 100 }
}

#[tauri::command]
fn set_market_hero_scale(scale: u32) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.market_hero_scale = scale.clamp(40, 100);
    save_config(&cfg)
}

/// 市场小卡片宽度
#[tauri::command]
fn get_market_card_size() -> u32 {
    let n = load_config().market_card_size;
    if (150..=320).contains(&n) { n } else { 210 }
}

#[tauri::command]
fn set_market_card_size(size: u32) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.market_card_size = size.clamp(150, 320);
    save_config(&cfg)
}

/// 获取市场「仅下载」的保存文件夹（未配置时返回系统默认下载文件夹）
#[tauri::command]
fn get_download_dir(app: tauri::AppHandle) -> Result<String, String> {
    let cfg = load_config();
    if !cfg.download_dir.trim().is_empty() {
        return Ok(cfg.download_dir);
    }
    // 回退到系统下载文件夹
    let dir = app
        .path()
        .download_dir()
        .map_err(|e| format!("获取系统下载文件夹失败: {e}"))?;
    Ok(dir.to_string_lossy().to_string())
}

/// 设置市场「仅下载」的保存文件夹（空 = 恢复系统默认）
#[tauri::command]
fn set_download_dir(path: String) -> Result<(), String> {
    let path = path.trim().to_string();
    if !path.is_empty() && !std::path::Path::new(&path).is_dir() {
        return Err(format!("目录不存在: {path}"));
    }
    let mut cfg = load_config();
    cfg.download_dir = path;
    save_config(&cfg)
}

#[tauri::command]
fn toggle_mod(character: String, mod_name: String, enabled: bool) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    toggle::set_mod_enabled(&path, &character, &mod_name, enabled)
}

#[tauri::command]
fn import(archive: String, character: String) -> Result<String, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    import_mod::import_mod(&path, &archive, &character)
}

#[tauri::command]
fn save_preset(name: String) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    preset::save_preset(&path, &name)
}

#[tauri::command]
fn apply_preset(name: String) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    preset::apply_preset(&path, &name)
}

#[tauri::command]
fn list_presets() -> Vec<config::Preset> {
    preset::list_presets()
}

#[tauri::command]
fn delete_preset(name: String) -> Result<(), String> {
    preset::delete_preset(&name)
}

#[tauri::command]
fn get_characters() -> Result<Vec<String>, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let groups = scan_mods(&path)?;
    Ok(groups.into_iter().map(|g| g.name).collect())
}

/// 读取封面图为 base64（data URL），供前端显示本地图片
/// 注意：仅供参考，前端已改用 asset 协议直接加载（避免 base64 撑爆 DOM）
#[allow(dead_code)]
#[tauri::command]
fn read_cover_base64(character: String, cover: String) -> Result<String, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let base = std::path::Path::new(&path);
    let img_path = base.join(&character).join(&cover);
    let bytes = std::fs::read(&img_path).map_err(|e| format!("读取封面失败: {e}"))?;
    let mime = if cover.to_lowercase().ends_with(".png") {
        "image/png"
    } else if cover.to_lowercase().ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    };
    let b64 = base64_encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// base64 解码（支持带 data:image/...;base64, 前缀）
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    // 去掉 data URL 前缀
    let s = match input.find("base64,") {
        Some(idx) => &input[idx + 7..],
        None => input,
    };
    let mut out: Vec<u8> = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return Err(format!("非法的 base64 字符: {}", c as char)),
        };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

// ===== 游戏卡片：重命名 / 自定义封面 / 内置游戏列表 =====

/// 内置游戏信息（用于「切换游戏」页展示全部支持的游戏）
#[derive(serde::Serialize)]
struct BuiltinGameInfo {
    id: String,
    name: String,
    /// 已配置的 Mods 路径（空字符串 = 未设置）
    mods_path: String,
    /// 是否已添加
    added: bool,
    /// 是否被用户隐藏
    hidden: bool,
}

/// 列出所有支持的内置游戏（含未设置路径的和被隐藏的）
#[tauri::command]
fn list_builtin_games() -> Vec<BuiltinGameInfo> {
    let cfg = load_config();
    BUILTIN_GAMES
        .iter()
        .map(|(_dir, id, default_name)| {
            let existing = cfg.games.iter().find(|g| g.id == *id);
            let id_s = id.to_string();
            let hidden = cfg.hidden_games.iter().any(|h| *h == id_s);
            BuiltinGameInfo {
                id: id.to_string(),
                name: existing
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| default_name.to_string()),
                mods_path: existing.map(|g| g.mods_path.clone()).unwrap_or_default(),
                added: existing.is_some(),
                hidden,
            }
        })
        .collect()
}

/// 隐藏内置游戏（不玩的游戏从列表里隐藏，可恢复）
#[tauri::command]
fn hide_builtin_game(id: String) -> Result<(), String> {
    if !BUILTIN_GAMES.iter().any(|(_, gid, _)| *gid == id) {
        return Err("仅支持隐藏内置游戏".into());
    }
    let mut cfg = load_config();
    if !cfg.hidden_games.iter().any(|h| *h == id) {
        cfg.hidden_games.push(id.clone());
    }
    // 已添加的也一并从游戏列表移除
    cfg.games.retain(|g| g.id != id);
    if cfg.active_game_id == id {
        cfg.active_game_id = cfg.games.first().map(|g| g.id.clone()).unwrap_or_default();
    }
    save_config(&cfg)
}

/// 恢复显示被隐藏的内置游戏
#[tauri::command]
fn unhide_builtin_game(id: String) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.hidden_games.retain(|h| *h != id);
    save_config(&cfg)
}

/// 设置/更改某个内置游戏的 Mods 路径（未添加则添加，已添加则更新路径）
#[tauri::command]
fn set_game_mods_path(id: String, name: String, mods_path: String) -> Result<(), String> {
    if !config::validate_mods_path(&mods_path) {
        return Err(format!("路径不含 mods: {mods_path}"));
    }
    // 路径含 mods 时，按其固化角色名单补齐目录结构
    characters::ensure_character_dirs(&id, &mods_path)?;
    let mut cfg = load_config();
    if let Some(g) = cfg.games.iter_mut().find(|g| g.id == id) {
        g.mods_path = mods_path;
    } else {
        cfg.games.push(Game {
            id: id.clone(),
            name,
            market_sources: std::collections::HashMap::new(),
            mods_path,
            presets: Vec::new(),
            sections: Vec::new(),
            mod_sections: std::collections::HashMap::new(),
            mod_notes: std::collections::HashMap::new(),
            pinned_mods: Vec::new(),
            character_order: Vec::new(),
            gb_game_id: 0,
            gb_category_id: 0,
            game_path: String::new(),
            launcher_path: String::new(),
            giml_path: String::new(),
            launch_mode: String::new(),
            launch_args: String::new(),
        });
        if cfg.active_game_id.is_empty() {
            cfg.active_game_id = id;
        }
    }
    save_config(&cfg)
}

/// 设置自定义游戏的 GameBanana 市场绑定（gb_game_id / gb_category_id，0 = 解除绑定）
#[tauri::command]
fn set_game_market_binding(id: String, gb_game_id: u32, gb_category_id: u32) -> Result<(), String> {
    let mut cfg = load_config();
    let game = cfg
        .games
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| format!("游戏不存在: {id}"))?;
    game.gb_game_id = gb_game_id;
    game.gb_category_id = gb_category_id;
    save_config(&cfg)
}

// ===== 启动游戏（复刻 QAQ Manager） =====

/// 获取游戏启动设置：有效路径 + 支持的模式 + 当前模式
#[tauri::command]
fn get_launch_config(id: String) -> Result<serde_json::Value, String> {
    let cfg = load_config();
    let game = cfg
        .games
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| format!("游戏不存在: {id}"))?;
    let (game_path, launcher_path) = launch::resolved_paths(&cfg, game);
    let giml_path = launch::resolved_giml_path(game);
    Ok(serde_json::json!({
        "game_path": game_path,
        "launcher_path": launcher_path,
        "giml_path": giml_path,
        "configured_game_path": game.game_path,
        "configured_launcher_path": game.launcher_path,
        "configured_giml_path": game.giml_path,
        "launch_mode": game.launch_mode,
        "launch_args": game.launch_args,
        "supported_modes": launch::supported_modes(&game.id),
        "importer": launch::importer_name(&game.id),
    }))
}

/// 全局 XXMI Launcher 路径（所有游戏共用；空 = 未设置）
#[tauri::command]
fn get_xxmi_launcher_path() -> String {
    let cfg = load_config();
    if !cfg.xxmi_launcher_path.trim().is_empty() {
        return cfg.xxmi_launcher_path.trim().to_string();
    }
    // 兼容旧配置：全局未设置时，展示当前游戏遗留的 launcher_path
    active_game(&cfg)
        .map(|g| g.launcher_path.trim().to_string())
        .unwrap_or_default()
}

/// 设置全局 XXMI Launcher 路径（所有游戏共用，一次设置即可）
#[tauri::command]
fn set_xxmi_launcher_path(path: String) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.xxmi_launcher_path = path.trim().to_string();
    save_config(&cfg)
}

/// 设置游戏启动路径/模式/参数（传空字符串表示清空，让系统重新推断）
#[tauri::command]
fn set_launch_config(
    id: String,
    game_path: String,
    launcher_path: String,
    giml_path: String,
    launch_mode: String,
    launch_args: String,
) -> Result<(), String> {
    let mut cfg = load_config();
    let game = cfg
        .games
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| format!("游戏不存在: {id}"))?;
    game.game_path = game_path.trim().to_string();
    game.launcher_path = launcher_path.trim().to_string();
    game.giml_path = giml_path.trim().to_string();
    game.launch_mode = launch_mode.trim().to_string();
    game.launch_args = launch_args.trim().to_string();
    save_config(&cfg)
}

/// 启动游戏（mode 为 None/空 = 用配置里的模式；"" 显式传 = 原版）
#[tauri::command]
fn launch_game(id: String, launch_mode: Option<String>) -> Result<(), String> {
    let cfg = load_config();
    let game = cfg
        .games
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| format!("游戏不存在: {id}"))?;
    launch::launch(&cfg, game, launch_mode.as_deref())
}

/// 重命名游戏（只改显示名，不改 id 和路径）
#[tauri::command]
fn rename_game(id: String, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("游戏名不能为空".into());
    }
    let mut cfg = load_config();
    let game = cfg
        .games
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or("游戏不存在")?;
    game.name = name;
    save_config(&cfg)
}

/// 设置游戏自定义封面（复制到应用数据目录的 game-covers/<id>.png）
#[tauri::command]
fn set_game_cover(id: String, image_path: String) -> Result<String, String> {
    let src = std::path::Path::new(&image_path);
    if !src.is_file() {
        return Err(format!("图片不存在: {image_path}"));
    }
    let dir = crate::config::config_dir().join("game-covers");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建封面目录失败: {e}"))?;
    let target = dir.join(format!("{id}.png"));
    std::fs::copy(src, &target).map_err(|e| format!("复制封面失败: {e}"))?;
    Ok(target.to_string_lossy().to_string())
}

/// 获取所有已自定义的游戏封面（game_id → 绝对路径）
#[tauri::command]
fn get_custom_game_covers() -> std::collections::HashMap<String, String> {
    let dir = crate::config::config_dir().join("game-covers");
    let mut map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() {
                if let Some(stem) = p.file_stem() {
                    map.insert(
                        stem.to_string_lossy().to_string(),
                        p.to_string_lossy().to_string(),
                    );
                }
            }
        }
    }
    map
}

/// 设置角色封面：把指定图片复制为角色目录下的 _cover.png
#[tauri::command]
fn set_character_cover(character: String, image_path: String) -> Result<String, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let char_dir = std::path::Path::new(&path).join(&character);
    if !char_dir.is_dir() {
        return Err(format!("角色目录不存在: {character}"));
    }
    let src = std::path::Path::new(&image_path);
    if !src.is_file() {
        return Err(format!("图片不存在: {image_path}"));
    }
    let bytes = std::fs::read(src).map_err(|e| format!("读取图片失败: {e}"))?;
    if bytes.is_empty() {
        return Err("图片内容为空".into());
    }
    let target = char_dir.join("_cover.png");
    std::fs::write(&target, &bytes).map_err(|e| format!("写入封面失败: {e}"))?;
    Ok("_cover.png".to_string())
}

/// 设置角色封面：接受 base64 图片数据（拖拽/粘贴场景）
#[tauri::command]
fn save_character_cover(character: String, base64_data: String) -> Result<String, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let char_dir = std::path::Path::new(&path).join(&character);
    if !char_dir.is_dir() {
        return Err(format!("角色目录不存在: {character}"));
    }
    let bytes = base64_decode(&base64_data)?;
    if bytes.is_empty() {
        return Err("图片数据为空".into());
    }
    let target = char_dir.join("_cover.png");
    std::fs::write(&target, &bytes).map_err(|e| format!("写入封面失败: {e}"))?;
    Ok("_cover.png".to_string())
}

/// 删除 Mod（移到回收站，可从回收站恢复）
#[tauri::command]
fn delete_mod(character: String, folder_name: String) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let dir = std::path::Path::new(&path).join(&character).join(&folder_name);
    if !dir.is_dir() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    // 清理分区归属记录
    let clean_name = folder_name
        .strip_prefix("DISABLED_")
        .unwrap_or(&folder_name)
        .to_string();
    let key = format!("{character}/{clean_name}");
    {
        let mut cfg2 = load_config();
        if let Some(game) = active_game_mut(&mut cfg2) {
            if game.mod_sections.remove(&key).is_some() {
                let _ = save_config(&cfg2);
            }
        }
    }
    // 移到回收站（可恢复）
    trash::delete(&dir).map_err(|e| format!("移到回收站失败: {e}"))?;
    Ok(())
}

/// 完整第三方组件清单（构建时嵌入源码，运行时不依赖外部文件是否存在）
const LICENSES_TXT: &str = include_str!("../resources/licenses.txt");

/// 返回完整第三方组件清单文本（关于页悬浮预览用）
#[tauri::command]
fn get_license_text() -> String {
    LICENSES_TXT.to_string()
}

/// 把完整清单写成 txt，并用系统默认程序打开；返回文件路径
#[tauri::command]
fn open_license_file() -> Result<String, String> {
    let dir = crate::config::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let path = dir.join("第三方组件清单.txt");
    std::fs::write(&path, LICENSES_TXT).map_err(|e| format!("写入文件失败: {e}"))?;
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| format!("打开文件失败: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// 用户确认退出后置真，放行 CloseRequested
static EXITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 拖动调整窗口时暂存的最新宽高（px），真正退出时写回配置，避免拖动过程频繁写盘
static WINDOW_W: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static WINDOW_H: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// 把内存里暂存的窗口尺寸写回配置（宽高为 0 表示未记录，跳过）
fn save_window_size() {
    let w = WINDOW_W.load(std::sync::atomic::Ordering::Relaxed);
    let h = WINDOW_H.load(std::sync::atomic::Ordering::Relaxed);
    if w < 200 || h < 200 {
        return; // 忽略过小/未记录的值，避免把最小化或异常尺寸写进去
    }
    let mut cfg = load_config();
    if cfg.window_width != w || cfg.window_height != h {
        cfg.window_width = w;
        cfg.window_height = h;
        let _ = save_config(&cfg);
    }
}

/// 检查应用更新：查询 GitHub Releases 最新版本并与当前版本比较
#[tauri::command]
async fn check_app_update() -> Result<update::UpdateInfo, String> {
    update::check().await
}

/// 真正退出应用（仅在用户点击「退出」后调用）
#[tauri::command]
fn exit_app(app: tauri::AppHandle) {
    EXITING.store(true, std::sync::atomic::Ordering::Relaxed);
    save_window_size();
    app.exit(0);
}

/// 当前进程是否以管理员权限运行。
/// 提权后 Windows（UIPI）会阻止从资源管理器把文件拖进窗口，且不会有任何拖放事件，
/// 前端据此提示用户以普通权限重启。
#[tauri::command]
fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut _,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            );
            CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 分析拖入的压缩包（不导入，仅预览）
#[tauri::command]
fn analyze_imports(paths: Vec<String>) -> Result<Vec<import_mod::ImportCandidate>, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    Ok(import_mod::analyze_imports(&path, &paths))
}

/// 批量导入项
#[derive(serde::Deserialize)]
struct BatchImportItem {
    path: String,
    character: String,
    name: String,
}

/// 批量导入
#[tauri::command]
fn batch_import(items: Vec<BatchImportItem>) -> Result<Vec<String>, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let mut done = Vec::new();
    let mut errors = Vec::new();
    for it in items {
        if it.character.trim().is_empty() {
            errors.push(format!("{}: 未选择角色", it.name));
            continue;
        }
        match import_mod::import_mod_named(&path, &it.path, &it.character, &it.name) {
            Ok(n) => done.push(n),
            Err(e) => errors.push(format!("{}: {e}", it.name)),
        }
    }
    if !errors.is_empty() && done.is_empty() {
        return Err(errors.join("\n"));
    }
    if !errors.is_empty() {
        // 部分成功：把错误附在结果里返回（前端只显示成功数）
        eprintln!("批量导入部分失败:\n{}", errors.join("\n"));
    }
    Ok(done)
}

// ===== 整合包导入（Mods/角色/具体Mod 结构） =====

/// 分析整合包：支持文件夹与 zip/7z/rar；返回角色/Mod 清单与冲突预检
#[tauri::command]
fn modpack_analyze(source: String) -> Result<modpack::ModpackInfo, String> {
    let cfg = load_config();
    let path = active_game(&cfg)
        .map(|g| g.mods_path.clone())
        .unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    modpack::analyze(&path, &source)
}

/// 执行整合包导入（mods_root/temp_dir 来自 modpack_analyze 的结果）
#[tauri::command]
fn modpack_import(
    app: tauri::AppHandle,
    mods_root: String,
    temp_dir: Option<String>,
    overwrite: bool,
) -> Result<modpack::ModpackResult, String> {
    let cfg = load_config();
    let path = active_game(&cfg)
        .map(|g| g.mods_path.clone())
        .unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    modpack::import(&app, &path, &mods_root, temp_dir, overwrite)
}

/// 切换识别层级：按指定目录重新构建清单（候选层级之间来回切）
#[tauri::command]
fn modpack_inspect(
    mods_root: String,
    temp_dir: Option<String>,
    source: Option<String>,
) -> Result<modpack::ModpackInfo, String> {
    let cfg = load_config();
    let path = active_game(&cfg)
        .map(|g| g.mods_path.clone())
        .unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let src = source.unwrap_or_else(|| mods_root.clone());
    modpack::inspect(&path, &mods_root, temp_dir, &src)
}

/// 清理整合包解压出来的临时目录（用户取消时调用）
#[tauri::command]
fn modpack_cleanup(path: String) -> Result<(), String> {
    modpack::cleanup(&path)
}

// ===== 角色皮肤分类（外观分区）命令 =====

/// 列出某角色的自定义分类
#[tauri::command]
fn list_sections(character: String) -> Vec<config::CharacterSection> {
    let cfg = load_config();
    active_game(&cfg)
        .map(|g| {
            g.sections
                .iter()
                .filter(|s| s.character == character)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 新增分类
#[tauri::command]
fn add_section(character: String, name: String) -> Result<config::CharacterSection, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("分类名称不能为空".into());
    }
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    // 同角色下不允许重名
    if game
        .sections
        .iter()
        .any(|s| s.character == character && s.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("分类「{name}」已存在"));
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let section = config::CharacterSection {
        id: format!("custom:{ts}"),
        character: character.clone(),
        name,
        created_at: ts.to_string(),
    };
    game.sections.push(section.clone());
    save_config(&cfg)?;
    Ok(section)
}

/// 新增角色（在 Mods 下创建角色文件夹）
#[tauri::command]
fn add_character(name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("角色名不能为空".into());
    }
    if name.chars().any(|c| "\\/:*?\"<>|".contains(c)) {
        return Err("角色名不能包含 \\ / : * ? \" < > |".into());
    }
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let dir = std::path::Path::new(&path).join(&name);
    if dir.exists() {
        return Err(format!("角色已存在：{name}"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建角色文件夹失败: {e}"))?;
    Ok(name)
}

/// 重命名角色（重命名文件夹，并同步更新预设、分区归属等配置）
#[tauri::command]
fn rename_character(old_name: String, new_name: String) -> Result<String, String> {
    let new_name = new_name.trim().to_string();
    if new_name.is_empty() {
        return Err("角色名不能为空".into());
    }
    if new_name.chars().any(|c| "\\/:*?\"<>|".contains(c)) {
        return Err("角色名不能包含 \\ / : * ? \" < > |".into());
    }
    if new_name == old_name {
        return Ok(new_name);
    }

    let mut cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let mods_root = std::path::Path::new(&path);
    let old_dir = mods_root.join(&old_name);
    let new_dir = mods_root.join(&new_name);
    if !old_dir.is_dir() {
        return Err(format!("角色目录不存在: {old_name}"));
    }
    if new_dir.exists() {
        return Err(format!("目标角色已存在: {new_name}"));
    }

    // 重命名文件夹
    std::fs::rename(&old_dir, &new_dir).map_err(|e| format!("重命名文件夹失败: {e}"))?;

    // 更新配置里的引用
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    // 1. 预设里的 rel_path： "旧角色/Mod名" → "新角色/Mod名"
    let old_prefix = format!("{old_name}/");
    let new_prefix = format!("{new_name}/");
    for p in game.presets.iter_mut() {
        for m in p.enabled_mods.iter_mut() {
            if m.starts_with(&old_prefix) {
                *m = format!("{new_prefix}{}", &m[old_prefix.len()..]);
            }
        }
    }
    // 2. mod 分区归属的 key
    let mut new_map = std::collections::HashMap::new();
    for (k, v) in game.mod_sections.iter() {
        if k.starts_with(&old_prefix) {
            new_map.insert(format!("{new_prefix}{}", &k[old_prefix.len()..]), v.clone());
        } else {
            new_map.insert(k.clone(), v.clone());
        }
    }
    game.mod_sections = new_map;
    // 3. 自定义分区的所属角色
    for s in game.sections.iter_mut() {
        if s.character == old_name {
            s.character = new_name.clone();
        }
    }
    // 4. 角色自定义顺序里的角色名
    for c in game.character_order.iter_mut() {
        if *c == old_name {
            *c = new_name.clone();
        }
    }
    save_config(&cfg)?;

    Ok(new_name)
}

/// 获取 Mod 归属分区映射（key = "角色/Mod名"，value = 分区 id）
#[tauri::command]
fn get_mod_sections() -> std::collections::HashMap<String, String> {
    let cfg = load_config();
    active_game(&cfg)
        .map(|g| g.mod_sections.clone())
        .unwrap_or_default()
}

// ===== Mod 备注 / 置顶 / 重命名 / 快捷键 =====

/// 解析 mod 目录：先按原名找，找不到试 DISABLED_ 前缀
fn resolve_mod_dir(
    mods_path: &str,
    character: &str,
    mod_name: &str,
) -> Result<std::path::PathBuf, String> {
    let base = std::path::Path::new(mods_path).join(character);
    let direct = base.join(mod_name);
    if direct.is_dir() {
        return Ok(direct);
    }
    let disabled = base.join(format!("DISABLED_{mod_name}"));
    if disabled.is_dir() {
        return Ok(disabled);
    }
    Err(format!("Mod 目录不存在: {character}/{mod_name}"))
}

#[tauri::command]
fn get_mod_notes() -> std::collections::HashMap<String, String> {
    let cfg = load_config();
    active_game(&cfg)
        .map(|g| g.mod_notes.clone())
        .unwrap_or_default()
}

#[tauri::command]
fn get_pinned_mods() -> Vec<String> {
    let cfg = load_config();
    active_game(&cfg)
        .map(|g| g.pinned_mods.clone())
        .unwrap_or_default()
}

/// 获取角色自定义顺序（角色名列表）
#[tauri::command]
fn get_character_order() -> Vec<String> {
    let cfg = load_config();
    active_game(&cfg)
        .map(|g| g.character_order.clone())
        .unwrap_or_default()
}

/// 设置角色自定义顺序（拖拽排序后保存）
#[tauri::command]
fn set_character_order(order: Vec<String>) -> Result<(), String> {
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    game.character_order = order;
    save_config(&cfg)
}

#[tauri::command]
fn set_mod_note(character: String, mod_name: String, note: String) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    resolve_mod_dir(&path, &character, &mod_name)?; // 校验存在
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    let key = format!("{character}/{mod_name}");
    if note.trim().is_empty() {
        game.mod_notes.remove(&key);
    } else {
        game.mod_notes.insert(key, note);
    }
    save_config(&cfg)
}

#[tauri::command]
fn toggle_mod_pin(character: String, mod_name: String) -> Result<bool, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    resolve_mod_dir(&path, &character, &mod_name)?;
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    let key = format!("{character}/{mod_name}");
    let pinned = if let Some(pos) = game.pinned_mods.iter().position(|p| p == &key) {
        game.pinned_mods.remove(pos);
        false
    } else {
        game.pinned_mods.push(key);
        true
    };
    save_config(&cfg)?;
    Ok(pinned)
}

#[tauri::command]
fn rename_mod(character: String, old_name: String, new_name: String) -> Result<String, String> {
    let new_name = new_name.trim().to_string();
    if new_name.is_empty() {
        return Err("Mod 名不能为空".into());
    }
    if new_name.chars().any(|c| "\\/:*?\"<>|".contains(c)) {
        return Err("Mod 名不能包含 \\ / : * ? \" < > |".into());
    }
    if new_name == old_name {
        return Ok(new_name);
    }

    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let old_dir = resolve_mod_dir(&path, &character, &old_name)?;
    // 保留 DISABLED_ 前缀
    let folder_name = old_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| old_name.clone());
    let prefix = folder_name
        .strip_prefix(&old_name)
        .unwrap_or("")
        .to_string(); // "DISABLED_" 或 ""
    let new_folder = format!("{prefix}{new_name}");
    let new_dir = old_dir.parent().unwrap().join(&new_folder);
    if new_dir.exists() {
        return Err(format!("目标 Mod 已存在: {new_folder}"));
    }

    std::fs::rename(&old_dir, &new_dir).map_err(|e| format!("重命名文件夹失败: {e}"))?;

    // 更新配置里的 rel_path 引用
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    let old_key = format!("{character}/{old_name}");
    let new_key = format!("{character}/{new_name}");
    // 1. 预设
    for p in game.presets.iter_mut() {
        for m in p.enabled_mods.iter_mut() {
            if m == &old_key {
                *m = new_key.clone();
            }
        }
    }
    // 2. 分区归属
    if let Some(v) = game.mod_sections.remove(&old_key) {
        game.mod_sections.insert(new_key.clone(), v);
    }
    // 3. 备注
    if let Some(v) = game.mod_notes.remove(&old_key) {
        game.mod_notes.insert(new_key.clone(), v);
    }
    // 4. 置顶
    if let Some(pos) = game.pinned_mods.iter().position(|p| p == &old_key) {
        game.pinned_mods[pos] = new_key.clone();
    }
    save_config(&cfg)?;

    Ok(new_name)
}

#[tauri::command]
fn get_mod_hotkeys(
    character: String,
    mod_name: String,
) -> Result<Vec<hotkeys::HotkeyEntry>, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let dir = resolve_mod_dir(&path, &character, &mod_name)?;
    Ok(hotkeys::get_mod_hotkeys(&dir))
}

#[tauri::command]
fn set_mod_hotkey(
    character: String,
    mod_name: String,
    section: String,
    new_key: String,
) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let dir = resolve_mod_dir(&path, &character, &mod_name)?;
    hotkeys::set_mod_hotkey(&dir, &section, &new_key)
}

/// 设置 Mod 归属分区（section_id 为 "base" 或空表示归回「原角色」）
#[tauri::command]
fn set_mod_section(
    character: String,
    mod_name: String,
    section_id: String,
) -> Result<(), String> {
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    let key = format!("{character}/{mod_name}");
    if section_id.is_empty() || section_id == "base" {
        game.mod_sections.remove(&key);
    } else {
        game.mod_sections.insert(key, section_id);
    }
    save_config(&cfg)
}

/// 删除分类（同时把该分类下的 Mod 归回「原角色」）
#[tauri::command]
fn delete_section(id: String) -> Result<(), String> {
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    let before = game.sections.len();
    game.sections.retain(|s| s.id != id);
    if game.sections.len() == before {
        return Err("分类不存在".into());
    }
    // 该分类下的 Mod 全部归回「原角色」
    game.mod_sections.retain(|_, v| v != &id);
    save_config(&cfg)
}

/// 重命名分类
#[tauri::command]
fn rename_section(id: String, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("分类名称不能为空".into());
    }
    let mut cfg = load_config();
    let game = active_game_mut(&mut cfg).ok_or("未配置游戏")?;
    let target_char = game
        .sections
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.character.clone())
        .ok_or("分类不存在")?;
    if game
        .sections
        .iter()
        .any(|s| s.character == target_char && s.id != id && s.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("分类「{name}」已存在"));
    }
    for s in game.sections.iter_mut() {
        if s.id == id {
            s.name = name;
            break;
        }
    }
    save_config(&cfg)
}

/// 互斥启用：启用目标 mod，并自动禁用**同一分区内**其他已启用的 mod（单选模式）
/// 不同分区的 mod 可以同时启用。
#[tauri::command]
fn enable_mod_exclusive(character: String, mod_name: String) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    // 目标 mod 所属分区（空字符串 = 原角色分区）
    let target_key = format!("{character}/{mod_name}");
    let target_section = active_game(&cfg)
        .and_then(|g| g.mod_sections.get(&target_key).cloned())
        .unwrap_or_default();

    let groups = scan_mods(&path)?;
    let group = groups
        .iter()
        .find(|g| g.name == character)
        .ok_or_else(|| format!("角色不存在: {character}"))?;

    // 只禁用「同一分区」内其他已启用的 mod
    for m in &group.mods {
        if m.name == mod_name || !m.enabled {
            continue;
        }
        let sec = active_game(&cfg)
            .and_then(|g| g.mod_sections.get(&m.rel_path).cloned())
            .unwrap_or_default();
        if sec == target_section {
            toggle::set_mod_enabled(&path, &character, &m.name, false)?;
        }
    }
    // 再启用目标
    toggle::set_mod_enabled(&path, &character, &mod_name, true)?;
    Ok(())
}

/// 同名 Mod 的另一种状态名（DISABLED_ 前缀 ↔ 无前缀），用于目标位置冲突检测
fn alternate_mod_name(name: &str) -> String {
    if name.starts_with("DISABLED_") {
        name[9..].to_string()
    } else {
        format!("DISABLED_{name}")
    }
}

/// 移动已安装的 Mod 到另一个角色目录（装错角色的纠正）：
/// 物理移动（fs::rename，同盘瞬时）保持启用/禁用状态不变，
/// 并同步迁移以 rel_path 为 key 的全部元数据（预设/分区/备注/置顶/市场来源）。
#[tauri::command]
fn move_mod(
    from_character: String,
    folder_name: String,
    to_character: String,
) -> Result<String, String> {
    if to_character == from_character {
        return Ok(folder_name);
    }
    if to_character.chars().any(|c| "\\/:*?\"<>|".contains(c)) {
        return Err("目标角色名含非法字符".into());
    }
    let mut cfg = load_config();
    let path = active_game(&cfg)
        .map(|g| g.mods_path.clone())
        .unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let mods_root = std::path::Path::new(&path);
    let src_dir = mods_root.join(&from_character).join(&folder_name);
    if !src_dir.is_dir() {
        return Err(format!("源 Mod 不存在: {from_character}/{folder_name}"));
    }
    let dst_char_dir = mods_root.join(&to_character);
    if !dst_char_dir.is_dir() {
        return Err(format!("目标角色目录不存在: {to_character}"));
    }
    // 同名冲突（含启用/禁用两种形态）
    if dst_char_dir.join(&folder_name).exists()
        || dst_char_dir.join(alternate_mod_name(&folder_name)).exists()
    {
        return Err(format!(
            "目标角色「{to_character}」已存在同名 Mod，请先处理同名 Mod 再移动"
        ));
    }

    // 物理移动（保持启用状态：目录名原样迁移）
    std::fs::rename(&src_dir, dst_char_dir.join(&folder_name))
        .map_err(|e| format!("移动目录失败: {e}"))?;

    // 元数据迁移：配置里的 key 都是干净名（rel_path = 角色/干净名）
    let clean = clean_name(&folder_name);
    let old_key = format!("{from_character}/{clean}");
    let new_key = format!("{to_character}/{clean}");
    if let Some(game) = active_game_mut(&mut cfg) {
        if let Some(v) = game.market_sources.remove(&old_key) {
            game.market_sources.insert(new_key.clone(), v);
        }
        if let Some(v) = game.mod_sections.remove(&old_key) {
            game.mod_sections.insert(new_key.clone(), v);
        }
        if let Some(v) = game.mod_notes.remove(&old_key) {
            game.mod_notes.insert(new_key.clone(), v);
        }
        for p in game.pinned_mods.iter_mut() {
            if *p == old_key {
                *p = new_key.clone();
            }
        }
        for preset in game.presets.iter_mut() {
            for m in preset.enabled_mods.iter_mut() {
                if *m == old_key {
                    *m = new_key.clone();
                }
            }
        }
        let _ = save_config(&cfg);
    }
    Ok(new_key)
}

/// 在资源管理器中打开 mod 文件夹
#[tauri::command]
fn open_mod_folder(character: String, folder_name: String) -> Result<(), String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let dir = std::path::Path::new(&path).join(&character).join(&folder_name);
    if !dir.is_dir() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {e}"))?;
    }
    Ok(())
}

/// 保存 Mod 预览图（把拖拽/剪贴板的图片写入 mod 目录的 _cover.png）
#[tauri::command]
fn save_mod_cover(
    character: String,
    folder_name: String,
    base64_data: String,
) -> Result<String, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let mod_dir = std::path::Path::new(&path).join(&character).join(&folder_name);
    if !mod_dir.is_dir() {
        return Err(format!("Mod 目录不存在: {}", mod_dir.display()));
    }
    let bytes = base64_decode(&base64_data)?;
    if bytes.is_empty() {
        return Err("图片数据为空".into());
    }
    let target = mod_dir.join("_cover.png");
    std::fs::write(&target, &bytes).map_err(|e| format!("写入预览图失败: {e}"))?;
    Ok("_cover.png".to_string())
}

// ===== Persist Bridge 命令 =====

/// 获取当前游戏的 persist 状态
#[tauri::command]
fn get_persist_status() -> PersistStatus {
    let cfg = load_config();
    PersistStatus {
        enabled: cfg.persist_enabled,
        cache_size: cfg.persist_cache_size,
    }
}

#[derive(serde::Serialize)]
struct PersistStatus {
    enabled: bool,
    cache_size: usize,
}

/// 设置 persist 开关
#[tauri::command]
fn set_persist_enabled(enabled: bool) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.persist_enabled = enabled;
    save_config(&cfg)?;

    // 对当前激活游戏执行迁移/还原
    if let Some(game) = active_game(&cfg) {
        let mods_path = game.mods_path.clone();
        if let Some(root) = std::path::Path::new(&mods_path).parent() {
            if enabled {
                // 启用：迁移当前所有启用的 mod + 挂载 includer
                persist::migrate_all_mods(root)?;
                persist::ensure_includer_mounted(root)?;
                persist::rebuild_active_includes(root)?;
            } else {
                // 禁用：还原所有 .qaqm-persistbak，清空缓存，并从 d3dx_user.ini / d3dx.ini 卸载挂载
                persist::restore_all_mods(root)?;
                persist::clear_cache(root)?;
                persist::unmount_includer(root)?;
            }
        }
    }
    Ok(())
}

/// 清空 persist 缓存
#[tauri::command]
fn clear_persist_cache() -> Result<usize, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    let Some(root) = std::path::Path::new(&path).parent() else {
        return Err("尚未配置 Mods 目录".into());
    };
    persist::clear_cache(root)
}

/// 设置 persist 缓存保留数量（0-50），并立即重建 include
#[tauri::command]
fn set_persist_cache_size(size: usize) -> Result<usize, String> {
    let clamped = size.min(persist::MAX_CACHE_SIZE);
    let mut cfg = load_config();
    cfg.persist_cache_size = clamped;
    save_config(&cfg)?;

    // 若 persist 已启用，立即按新数量重建 include（让设置马上生效）
    if cfg.persist_enabled {
        if let Some(game) = active_game(&cfg) {
            let path = game.mods_path.clone();
            if let Some(root) = std::path::Path::new(&path).parent() {
                let _ = persist::rebuild_active_includes(root);
            }
        }
    }
    Ok(clamped)
}

/// 读取某个 mod 的 persist 状态（变量名 → 当前值）
#[tauri::command]
fn get_mod_persist_state(character: String, mod_name: String) -> Result<Vec<(String, String)>, String> {
    let cfg = load_config();
    let path = active_game(&cfg).map(|g| g.mods_path.clone()).unwrap_or_default();
    if path.is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let mods_root = std::path::Path::new(&path);
    let mod_dir = mods_root.join(&character).join(&mod_name);
    persist::read_mod_persist_values(mods_root, &mod_dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 单实例：重复启动时在创建窗口前拦下，弹系统提示并退出。
    // 必须放在 Builder 之前 —— 一旦创建第二个 WebView2，就会共用数据目录，
    // 导致已有实例的输入框全部失效。
    if instance::init() {
        instance::warn_duplicate_and_exit("endfield-mod-manager");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(market::CancelRegistry(std::sync::Mutex::new(std::collections::HashMap::new())))
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if !EXITING.load(std::sync::atomic::Ordering::Relaxed) {
                    // 拦下关闭，交给前端弹二次确认
                    api.prevent_close();
                    let _ = tauri::Emitter::emit(window, "request-exit", ());
                } else {
                    // 真正退出前，把最后记住的窗口尺寸写回配置
                    save_window_size();
                }
            }
            // 拖动调整窗口大小时，把最新尺寸暂存到内存（不频繁写盘）
            if let tauri::WindowEvent::Resized(size) = event {
                WINDOW_W.store(size.width.max(1), std::sync::atomic::Ordering::Relaxed);
                WINDOW_H.store(size.height.max(1), std::sync::atomic::Ordering::Relaxed);
            }
        })
        .setup(|app| {
            // 恢复上次关闭时的窗口大小
            use tauri::Manager;
            let cfg = load_config();
            if cfg.window_width >= 200 && cfg.window_height >= 200 {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_size(tauri::LogicalSize::new(
                        cfg.window_width,
                        cfg.window_height,
                    ));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_games,
            get_active_game_id,
            set_active_game,
            add_game,
            remove_game,
            get_mods_path,
            set_mods_path,
            scan,
            get_character_gallery,
            get_single_mode,
            set_single_mode,
            get_card_size,
            get_download_dir,
            set_download_dir,
            set_card_size,
            get_gallery_card_size,
            set_gallery_card_size,
            get_market_hero_scale,
            set_market_hero_scale,
            get_market_card_size,
            set_market_card_size,
            toggle_mod,
            import,
            save_preset,
            apply_preset,
            list_presets,
            delete_preset,
            get_characters,
            read_cover_base64,
            save_mod_cover,
            open_mod_folder,
            move_mod,
            enable_mod_exclusive,
            list_sections,
            add_section,
            delete_section,
            rename_section,
            get_mod_sections,
            set_mod_section,
            get_mod_notes,
            get_pinned_mods,
            get_character_order,
            set_character_order,
            set_mod_note,
            toggle_mod_pin,
            rename_mod,
            get_mod_hotkeys,
            set_mod_hotkey,
            rename_character,
            add_character,
            analyze_imports,
            is_elevated,
            exit_app,
            get_license_text,
            open_license_file,
            check_app_update,
            batch_import,
            modpack_analyze,
            modpack_inspect,
            modpack_import,
            modpack_cleanup,
            delete_mod,
            set_character_cover,
            save_character_cover,
            rename_game,
            set_game_cover,
            get_custom_game_covers,
            list_builtin_games,
            set_game_mods_path,
            set_game_market_binding,
            get_launch_config,
            set_launch_config,
            get_xxmi_launcher_path,
            set_xxmi_launcher_path,
            launch_game,
            hide_builtin_game,
            unhide_builtin_game,
            get_persist_status,
            set_persist_enabled,
            clear_persist_cache,
            set_persist_cache_size,
            get_mod_persist_state,
            market_categories,
            market_mods,
            market_search,
            market_mod_detail,
            market_comments,
            market_top_subs,
            market_download,
            market_download_file,
            cancel_market_download,
            market_check_updates
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

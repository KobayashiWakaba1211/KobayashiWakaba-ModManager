//! GameBanana 市场功能（复刻 IMM 在线模式）
//!
//! 终末地在 GameBanana 上：game id = 21842，角色分类根节点 = 42770。
//! API 端点（apiv11）：
//!   - 分类:     Mod/Categories?_idCategoryRow=42770&_sSort=a_to_z&_bShowEmpty=true
//!   - 分类列表: Mod/Index?_nPerpage=N&_aFilters[Generic_Category]=<cat>&_sSort=<sort>&_nPage=P
//!   - 首页流:   Game/21842/Subfeed?_csvModelInclusions=Mod&_sSort=default&_nPage=P
//!   - 搜索:     Util/Search/Results?_sModelName=Mod&_sOrder=best_match&_idGameRow=21842&_sSearchString=<q>&_nPage=P
//!   - 详情:     Mod/<id>/ProfilePage
//!   - 下载:     https://gamebanana.com/dl/<fileId>

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

const GB_API: &str = "https://gamebanana.com/apiv11/";

/// 市场列表每页数量（分类列表 / 首页流 / 搜索统一）
/// 足够大让宽屏下网格铺满（每行约 8-12 个，60 个约 5-8 行）
pub const MARKET_PER_PAGE: u32 = 60;

/// GameBanana Mod/Index 分类查询的硬上限：_nPerpage=60 会返回空列表，
/// 带排序时更是直接 400。首页 Subfeed 与搜索端点不受此限制，仍用 MARKET_PER_PAGE。
const GB_INDEX_PER_PAGE: u32 = 50;

/// Mod/Index / Subfeed 允许的排序别名（其余会报 UNKNOWN_SORT）
const VALID_SORTS: &[&str] = &[
    "",
    "default",
    "Generic_MostLiked",
    "Generic_MostViewed",
    "Generic_MostDownloaded",
];

async fn gb_get_json(url: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    gb_get_json_with(&client, url).await
}

/// 复用外部 client 的 GB JSON 请求（更新检测批量调用时共享连接）
async fn gb_get_json_with(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let resp = client
        .get(url)
        .header("User-Agent", "endfield-mod-manager/0.1")
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GameBanana 返回 HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("解析 JSON 失败: {e}"))
}

// ===== 数据结构 =====

#[derive(Serialize, Clone)]
pub struct MarketCategory {
    pub id: u32,
    pub name: String,
    pub item_count: u32,
    pub icon_url: String,
    /// 分组："character" = 角色分类，"general" = 通用分类（武器/UI/其他等）
    pub group: String,
}

#[derive(Serialize, Clone)]
pub struct MarketMod {
    pub id: u32,
    pub name: String,
    pub url: String,
    pub author: String,
    pub image: String,
    pub likes: i64,
    pub views: i64,
    pub date_added: i64,
}

#[derive(Serialize)]
pub struct MarketModList {
    pub records: Vec<MarketMod>,
    pub total: u32,
    pub page: u32,
    pub has_more: bool,
}

#[derive(Serialize, Clone)]
pub struct MarketFile {
    pub id: u32,
    pub filename: String,
    pub size: u64,
    pub download_url: String,
}

#[derive(Serialize)]
pub struct MarketModDetail {
    pub id: u32,
    pub name: String,
    pub url: String,
    pub author: String,
    pub text: String,
    pub images: Vec<String>,
    pub files: Vec<MarketFile>,
    pub likes: i64,
    pub views: i64,
    pub downloads: i64,
    pub category: String,
}

// ===== 解析辅助 =====

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn i64_field(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0)
}

/// 从预览媒体对象里挑第一张截图的完整 URL（优先 530 缩略，其次原图）
fn preview_image(v: &Value, big: bool) -> String {
    let images = v
        .get("_aPreviewMedia")
        .and_then(|m| m.get("_aImages"))
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    for img in &images {
        if img.get("_sType").and_then(|x| x.as_str()) == Some("video") {
            continue;
        }
        let base = str_field(img, "_sBaseUrl");
        let file = if big {
            str_field(img, "_sFile")
        } else {
            let f530 = str_field(img, "_sFile530");
            if !f530.is_empty() {
                f530
            } else {
                str_field(img, "_sFile")
            }
        };
        if !base.is_empty() && !file.is_empty() {
            return format!("{base}/{file}");
        }
    }
    String::new()
}

fn parse_mod_record(v: &Value) -> Option<MarketMod> {
    let id = v.get("_idRow")?.as_u64()? as u32;
    let author = v
        .get("_aSubmitter")
        .map(|s| str_field(s, "_sName"))
        .unwrap_or_default();
    Some(MarketMod {
        id,
        name: str_field(v, "_sName"),
        url: str_field(v, "_sProfileUrl"),
        author,
        image: preview_image(v, false),
        likes: i64_field(v, "_nLikeCount"),
        views: i64_field(v, "_nViewCount"),
        date_added: i64_field(v, "_tsDateAdded"),
    })
}

fn parse_index_response(v: Value) -> MarketModList {
    let total = v
        .get("_aMetadata")
        .and_then(|m| m.get("_nRecordCount"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    let records: Vec<MarketMod> = v
        .get("_aRecords")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(parse_mod_record).collect())
        .unwrap_or_default();
    MarketModList {
        total,
        records,
        page: 1,
        has_more: false,
    }
}

// ===== Tauri 命令 =====

/// 市场游戏注册表：游戏 id -> (GameBanana 游戏 id, 分类根节点 id)
/// 复刻 IMM 的多游戏市场机制：切游戏后市场跟随切换到对应游戏
pub const GAME_GB_REGISTRY: &[(&str, u32, u32)] = &[
    ("endfield", 21842, 42770),   // 明日方舟：终末地
    ("starrail", 18366, 22832),   // 崩坏：星穹铁道
    ("wuthering", 20357, 29524),  // 鸣潮
    ("zenless", 19567, 30305),    // 绝区零
    ("GenshinImpact", 8552, 18140), // 原神
];

/// 各游戏的通用分类（武器 / UI / 其他等非角色分类），复刻 IMM gameData.json 的 generic.categories。
/// GameBanana 的 Mod/Categories 只返回角色分类，这些通用分类需要固化补上，否则市场里看不到武器等 Mod。
/// 格式：(分类 id, 分类名, 条目数)
pub const GAME_GENERIC_CATEGORIES: &[(&str, &[(u32, &str, u32)])] = &[
    ("endfield", &[
        (42777, "Arts Unit", 0),
        (42774, "Great Sword", 0),
        (42776, "Handcannon", 0),
        (42775, "Polearm", 0),
        (42773, "Sword", 0),
        (42706, "UI", 6),
        (42780, "Other", 4),
    ]),
    ("starrail", &[
        (22833, "Weapons", 29),
        (22830, "UI", 103),
        (22628, "Other", 85),
        (22829, "Objects", 25),
        (23974, "Entity", 21),
    ]),
    ("wuthering", &[
        // IMM 把 31838 (NPCs & Entities) 过滤掉了，这里保持一致
        (29493, "Other", 75),
        (29496, "UI", 55),
    ]),
    ("zenless", &[
        (30702, "Bangboo", 34),
        (29874, "Other", 194),
        (30395, "UI", 211),
    ]),
    ("GenshinImpact", &[
        (26229, "Bows", 63),
        (26228, "Catalysts", 69),
        (26227, "Claymores", 108),
        (26225, "Polearms", 121),
        (26226, "Swords", 200),
        (23574, "Gadget", 46),
        (24279, "Waverider", 11),
        (12526, "Other", 197),
        (22474, "UI", 197),
        (18310, "Objects", 184),
        (22725, "Entity", 60),
    ]),
];

fn generic_categories_for(game_id: &str) -> &'static [(u32, &'static str, u32)] {
    GAME_GENERIC_CATEGORIES
        .iter()
        .find(|(id, _)| *id == game_id)
        .map(|(_, cats)| *cats)
        .unwrap_or(&[])
}

/// 按当前激活游戏查 GameBanana id：
/// 1) 内置注册表优先；2) 用户自定义绑定兜底；3) 都没有则给出明确提示去绑定。
fn gb_ids_for_game(game_id: &str) -> Result<(u32, u32), String> {
    if let Some((_, gb, cat)) = GAME_GB_REGISTRY.iter().find(|(id, _, _)| *id == game_id) {
        return Ok((*gb, *cat));
    }
    let cfg = crate::config::load_config();
    if let Some(g) = cfg.games.iter().find(|g| g.id == game_id) {
        if g.gb_game_id > 0 {
            // 分类根未填时用 0，market_categories 会跳过分类过滤直接展示可用分类
            return Ok((g.gb_game_id, g.gb_category_id));
        }
    }
    Err(format!(
        "游戏「{game_id}」尚未绑定 GameBanana 市场：请到「切换游戏」页点该游戏的「绑定市场」填写 GameBanana 游戏 ID"
    ))
}

/// 获取当前激活游戏的全部分类（GameBanana 角色分类 + IMM 固化的通用分类）
#[tauri::command]
pub async fn market_categories() -> Result<Vec<MarketCategory>, String> {
    let cfg = crate::config::load_config();
    let (gb_game_id, root_category) = gb_ids_for_game(&cfg.active_game_id)?;
    // 绑定了分类根则用 _idCategoryRow（角色分类列表）；未填分类根则按游戏 id 列顶层分类
    let url = if root_category > 0 {
        format!("{GB_API}Mod/Categories?_idCategoryRow={root_category}&_sSort=a_to_z&_bShowEmpty=true")
    } else {
        format!("{GB_API}Mod/Categories?_idGameRow={gb_game_id}&_sSort=a_to_z&_bShowEmpty=true")
    };
    let v = gb_get_json(&url).await?;
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut chars: Vec<MarketCategory> = arr
        .iter()
        .filter_map(|c| {
            let id = c.get("_idRow")?.as_u64()? as u32;
            Some(MarketCategory {
                id,
                name: str_field(c, "_sName"),
                item_count: c.get("_nItemCount").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                icon_url: str_field(c, "_sIconUrl"),
                group: "character".into(),
            })
        })
        .collect();
    chars.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    // 追加通用分类（武器 / UI / 其他等），与 IMM 一致；已被 API 返回的 id 跳过避免重复
    let existing: std::collections::HashSet<u32> = chars.iter().map(|c| c.id).collect();
    let mut general: Vec<MarketCategory> = generic_categories_for(&cfg.active_game_id)
        .iter()
        .filter(|(id, _, _)| !existing.contains(id))
        .map(|(id, name, count)| MarketCategory {
            id: *id,
            name: name.to_string(),
            item_count: *count,
            icon_url: String::new(),
            group: "general".into(),
        })
        .collect();
    general.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    chars.extend(general);
    Ok(chars)
}

/// 首页 TopSubs 轮播推荐（复刻 IMM 的 banner）
#[derive(Serialize, Clone)]
pub struct MarketTopSub {
    pub id: u32,
    pub name: String,
    pub url: String,
    pub image: String,
    pub author: String,
    pub likes: i64,
    pub period: String,
}

#[tauri::command]
pub async fn market_top_subs() -> Result<Vec<MarketTopSub>, String> {
    let cfg = crate::config::load_config();
    let (gb_game_id, _) = gb_ids_for_game(&cfg.active_game_id)?;
    let url = format!("{GB_API}Game/{gb_game_id}/TopSubs");
    let v = gb_get_json(&url).await?;
    let arr = v.as_array().cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|x| {
            let id = x.get("_idRow")?.as_u64()? as u32;
            let model = str_field(x, "_sModelName");
            // IMM 只展示 Mod 类型（且不过滤 hide 由前端处理）
            if model != "Mod" {
                return None;
            }
            Some(MarketTopSub {
                id,
                name: str_field(x, "_sName"),
                url: str_field(x, "_sProfileUrl"),
                image: str_field(x, "_sImageUrl"),
                author: x
                    .get("_aSubmitter")
                    .map(|s| str_field(s, "_sName"))
                    .unwrap_or_default(),
                likes: i64_field(x, "_nLikeCount"),
                period: str_field(x, "_sPeriod"),
            })
        })
        .collect())
}

/// 评论（复刻 IMM 的 Mod/Posts）
#[derive(Serialize, Clone)]
pub struct MarketComment {
    pub id: u32,
    pub author: String,
    pub avatar: String,
    pub text: String,
    pub date_added: i64,
}

#[tauri::command]
pub async fn market_comments(mod_id: u32) -> Result<Vec<MarketComment>, String> {
    let url = format!("{GB_API}Mod/{mod_id}/Posts?_nPage=1&_nPerpage=15&_sSort=popular");
    let v = gb_get_json(&url).await?;
    let arr = v
        .get("_aRecords")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|x| {
            let id = x.get("_idRow")?.as_u64()? as u32;
            let text = str_field(x, "_sText");
            if text.eq_ignore_ascii_case("[trashed]") || text.is_empty() {
                return None; // 过滤已删除的评论
            }
            let poster = x.get("_aPoster");
            Some(MarketComment {
                id,
                author: poster
                    .map(|p| str_field(p, "_sName"))
                    .unwrap_or_default(),
                avatar: poster
                    .map(|p| str_field(p, "_sAvatarUrl"))
                    .unwrap_or_default(),
                text,
                date_added: i64_field(x, "_tsDateAdded"),
            })
        })
        .collect())
}

/// 按分类取 Mod 列表（category_id = 0 表示首页订阅流）。市场随当前激活游戏切换。
#[tauri::command]
pub async fn market_mods(category_id: u32, sort: String, page: u32) -> Result<MarketModList, String> {
    let sort = if VALID_SORTS.contains(&sort.as_str()) {
        sort
    } else {
        String::new()
    };
    let page = page.max(1);
    let cfg = crate::config::load_config();
    let (gb_game_id, _) = gb_ids_for_game(&cfg.active_game_id)?;
    if category_id == 0 {
        // Subfeed 端点固定 15 条/页（不支持 _nPerpage），为铺满网格拼接多页：
        // 前端页码 page 对应 Subfeed 的 [page, page+N) 连续页，合并去重
        const SUBFEED_PAGE_SIZE: u32 = 15;
        let pages_to_fetch = MARKET_PER_PAGE.div_ceil(SUBFEED_PAGE_SIZE);
        let mut all: Vec<MarketMod> = Vec::new();
        let mut last_page_full = false;
        for p in page..page + pages_to_fetch {
            let url = format!(
                "{GB_API}Game/{gb_game_id}/Subfeed?_csvModelInclusions=Mod&_sSort=default&_nPage={p}"
            );
            let list = parse_index_response(gb_get_json(&url).await?);
            last_page_full = list.records.len() == SUBFEED_PAGE_SIZE as usize;
            for r in list.records {
                // Subfeed 可能出现重复条目（同 Mod 多文件更新），按 id 去重
                if !all.iter().any(|x| x.id == r.id) {
                    all.push(r);
                }
            }
            if !last_page_full {
                break; // 后面没有更多页了
            }
        }
        let has_more = last_page_full && all.len() >= MARKET_PER_PAGE as usize;
        let total = all.len() as u32;
        return Ok(MarketModList {
            records: all,
            total,
            page,
            has_more,
        });
    }
    let url = format!(
        "{GB_API}Mod/Index?_nPerpage={GB_INDEX_PER_PAGE}&_aFilters%5BGeneric_Category%5D={category_id}&_sSort={sort}&_nPage={page}"
    );
    let mut list = parse_index_response(gb_get_json(&url).await?);
    list.page = page;
    // 满页即认为可能还有下一页（最后一页恰好满页时多翻一次会得到空页，可接受）
    list.has_more = list.records.len() == GB_INDEX_PER_PAGE as usize;
    Ok(list)
}

/// 全局搜索当前激活游戏的 Mod
#[tauri::command]
pub async fn market_search(term: String, page: u32) -> Result<MarketModList, String> {
    let page = page.max(1);
    let term = term.trim();
    if term.is_empty() {
        return Ok(MarketModList {
            records: vec![],
            total: 0,
            page: 1,
            has_more: false,
        });
    }
    let cfg = crate::config::load_config();
    let (gb_game_id, _) = gb_ids_for_game(&cfg.active_game_id)?;
    let url = format!(
        "{GB_API}Util/Search/Results?_sModelName=Mod&_sOrder=best_match&_idGameRow={gb_game_id}&_sSearchString={}&_nPerpage={MARKET_PER_PAGE}&_nPage={page}",
        urlencoding_lite(term)
    );
    let mut list = parse_index_response(gb_get_json(&url).await?);
    list.page = page;
    list.has_more = list.records.len() == MARKET_PER_PAGE as usize;
    Ok(list)
}

/// 有更新的本地 Mod（更新检测结果项）
#[derive(Debug, Clone, Serialize)]
pub struct ModUpdateInfo {
    /// rel_path（角色/Mod 目录名）
    pub rel_path: String,
    /// GameBanana mod id
    pub gb_mod_id: u32,
    /// GameBanana 页面链接
    pub page_url: String,
    /// GB 侧最新文件时间（Unix 毫秒）
    pub latest_ts: i64,
    /// 本地安装时间（Unix 毫秒）
    pub installed_at: i64,
}

/// 检测当前激活游戏里「市场安装的 Mod」是否有新版本：
/// 对每个有来源记录的 Mod 调 GameBanana ProfilePage，取全部文件的最大时间戳，
/// 大于本地安装时间 → 有更新。顺序请求（GB API 无批量端点），带 100ms 间隔限速。
#[tauri::command]
pub async fn market_check_updates() -> Result<Vec<ModUpdateInfo>, String> {
    let cfg = crate::config::load_config();
    let game = cfg
        .games
        .iter()
        .find(|g| g.id == cfg.active_game_id)
        .ok_or_else(|| "未选择游戏".to_string())?;
    let sources: Vec<(String, u32, String, i64)> = game
        .market_sources
        .iter()
        .map(|(k, v)| (k.clone(), v.gb_mod_id, v.page_url.clone(), v.installed_at))
        .collect();
    if sources.is_empty() {
        return Ok(vec![]);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let mut out: Vec<ModUpdateInfo> = Vec::new();
    for (rel_path, gb_mod_id, page_url, installed_at) in &sources {
        let url = format!("{GB_API}Mod/{gb_mod_id}/ProfilePage");
        let ok = match gb_get_json_with(&client, &url).await {
            Ok(v) => v,
            Err(_) => continue, // 单个失败不影响整体
        };
        // 取该 mod 全部文件的最大时间戳（秒 → 毫秒）
        let mut latest: i64 = 0;
        if let Some(files) = ok.get("_aFiles").and_then(|f| f.as_array()) {
            for file in files {
                let ts = file
                    .get("_tsDateModified")
                    .or_else(|| file.get("_tsDateAdded"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0)
                    * 1000;
                if ts > latest {
                    latest = ts;
                }
            }
        }
        if latest > *installed_at {
            out.push(ModUpdateInfo {
                rel_path: rel_path.clone(),
                gb_mod_id: *gb_mod_id,
                page_url: page_url.clone(),
                latest_ts: latest,
                installed_at: *installed_at,
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await; // 限速
    }
    Ok(out)
}

/// Mod 详情（描述 HTML、截图、文件列表）
#[tauri::command]
pub async fn market_mod_detail(mod_id: u32) -> Result<MarketModDetail, String> {
    let url = format!("{GB_API}Mod/{mod_id}/ProfilePage");
    let v = gb_get_json(&url).await?;
    let images: Vec<String> = v
        .get("_aPreviewMedia")
        .and_then(|m| m.get("_aImages"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|img| img.get("_sType").and_then(|x| x.as_str()) == Some("screenshot"))
                .filter_map(|img| {
                    let base = str_field(img, "_sBaseUrl");
                    let file = str_field(img, "_sFile");
                    if base.is_empty() || file.is_empty() {
                        None
                    } else {
                        Some(format!("{base}/{file}"))
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let files: Vec<MarketFile> = v
        .get("_aFiles")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    let id = f.get("_idRow")?.as_u64()? as u32;
                    Some(MarketFile {
                        id,
                        filename: str_field(f, "_sFile"),
                        size: f.get("_nFilesize").and_then(|x| x.as_u64()).unwrap_or(0),
                        download_url: str_field(f, "_sDownloadUrl"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(MarketModDetail {
        id: v.get("_idRow").and_then(|x| x.as_u64()).unwrap_or(mod_id as u64) as u32,
        name: str_field(&v, "_sName"),
        url: str_field(&v, "_sProfileUrl"),
        author: v
            .get("_aSubmitter")
            .map(|s| str_field(s, "_sName"))
            .unwrap_or_default(),
        text: str_field(&v, "_sText"),
        images,
        files,
        likes: i64_field(&v, "_nLikeCount"),
        views: i64_field(&v, "_nViewCount"),
        downloads: i64_field(&v, "_nDownloadCount"),
        category: v
            .get("_aCategory")
            .map(|c| str_field(c, "_sName"))
            .unwrap_or_default(),
    })
}

/// 下载任务取消标志注册表（taskId → 取消标志）
pub struct CancelRegistry(pub Mutex<HashMap<String, Arc<AtomicBool>>>);

/// 取消一个下载任务（设置取消标志，下载循环检测到后中断并清理）
#[tauri::command]
pub fn cancel_market_download(
    state: tauri::State<CancelRegistry>,
    task_id: String,
) -> Result<(), String> {
    if let Some(flag) = state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .get(&task_id)
    {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// 下载市场 Mod 并安装到指定角色目录（带进度事件 market-dl-progress）
/// cover_url 非空时，安装成功后把该图保存为 mod 封面（_cover.png / _cover.jpg）
/// task_id：前端生成的任务标识，用于进度事件回传与取消
#[tauri::command]
pub async fn market_download(
    app: AppHandle,
    state: tauri::State<'_, CancelRegistry>,
    mods_path: String,
    download_url: String,
    filename: String,
    character: String,
    mod_name: String,
    cover_url: Option<String>,
    task_id: String,
    gb_mod_id: u32,
    page_url: String,
) -> Result<String, String> {
    let filename = sanitize_filename(&filename);
    if filename.is_empty() {
        return Err("无效的文件名".into());
    }
    let character = sanitize_filename(&character);
    if character.is_empty() {
        return Err("无效的目标角色目录".into());
    }
    let mod_name_raw = sanitize_filename(&mod_name);
    let mod_name = if mod_name_raw.is_empty() {
        PathBuf::from(&filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "mod".into())
    } else {
        mod_name_raw
    };

    // 注册取消标志（任务结束移除）
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .insert(task_id.clone(), cancel.clone());

    // 下载到临时文件
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let resp = client
        .get(&download_url)
        .header("User-Agent", "endfield-mod-manager/0.1")
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败：HTTP {}", resp.status()));
    }
    let total = resp.content_length();

    let tmp_path = std::env::temp_dir().join(format!(
        "efmi-dl-{}-{}-{filename}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let mut file = std::fs::File::create(&tmp_path).map_err(|e| format!("创建临时文件失败: {e}"))?;
    let mut downloaded: u64 = 0;
    let mut resp = resp;
    let mut cancelled = false;
    // 用 timeout 轮询读 chunk：网络停滞时也能每 2 秒醒来检查取消标志，
    // 否则 chunk().await 一直阻塞会导致「取消无效、任务卡死」
    loop {
        let chunk = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resp.chunk(),
        )
        .await
        {
            Err(_elapsed) => {
                // 2 秒无数据：检查取消后继续等
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
                continue;
            }
            Ok(Err(e)) => {
                let err = format!("下载数据失败: {e}");
                drop(file);
                let _ = std::fs::remove_file(&tmp_path);
                state
                    .0
                    .lock()
                    .map_err(|e| e.to_string())?
                    .remove(&task_id);
                return Err(err);
            }
            Ok(Ok(None)) => break, // 下载完成
            Ok(Ok(Some(c))) => c,
        };
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        file.write_all(&chunk)
            .map_err(|e| format!("写入临时文件失败: {e}"))?;
        downloaded += chunk.len() as u64;
        let _ = app.emit(
            "market-dl-progress",
            serde_json::json!({
                "taskId": task_id,
                "filename": filename,
                "downloaded": downloaded,
                "total": total,
            }),
        );
    }
    drop(file);
    state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&task_id);

    if cancelled {
        let _ = std::fs::remove_file(&tmp_path);
        return Err("下载已取消".into());
    }

    if downloaded == 0 {
        let _ = std::fs::remove_file(&tmp_path);
        return Err("下载内容为空".into());
    }

    // 复用导入逻辑：解压安装到 mods/<角色>/<Mod名>
    let result = crate::import_mod::import_mod_named(&mods_path, &tmp_path.to_string_lossy(), &character, &mod_name);
    let _ = std::fs::remove_file(&tmp_path);
    let installed = result?;

    // 尽力而为：把下载页预览图保存为 mod 封面（失败不影响安装结果）
    if let Some(cover_url) = cover_url {
        if !cover_url.trim().is_empty() {
            let mod_dir = std::path::Path::new(&mods_path).join(&character).join(&installed);
            if let Err(e) = save_mod_cover_image(&cover_url, &mod_dir).await {
                let _ = app.emit(
                    "market-dl-progress",
                    serde_json::json!({ "filename": format!("{installed} 封面"), "downloaded": 0, "total": 0, "cover_error": e }),
                );
            }
        }
    }

    // 记录安装来源（更新检测用）：写入当前激活游戏的 market_sources
    {
        let mut cfg = crate::config::load_config();
        if let Some(game) = cfg.games.iter_mut().find(|g| g.id == cfg.active_game_id) {
            let rel_path = format!("{character}/{installed}");
            game.market_sources.insert(
                rel_path,
                crate::config::MarketSourceRecord {
                    gb_mod_id,
                    page_url,
                    installed_at: (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0)),
                },
            );
            let _ = crate::config::save_config(&cfg);
        }
    }

    Ok(format!("{character}/{installed}"))
}

/// 下载市场 Mod 文件到指定文件夹（仅下载，不安装；带进度事件 market-dl-progress）
/// 重名文件自动追加序号。返回最终保存路径。
#[tauri::command]
pub async fn market_download_file(
    app: AppHandle,
    state: tauri::State<'_, CancelRegistry>,
    download_url: String,
    filename: String,
    save_dir: String,
    task_id: String,
) -> Result<String, String> {
    let filename = sanitize_filename(&filename);
    if filename.is_empty() {
        return Err("无效的文件名".into());
    }
    let save_dir = save_dir.trim().to_string();
    if save_dir.is_empty() {
        return Err("下载文件夹未设置".into());
    }
    let dir = PathBuf::from(&save_dir);
    if !dir.is_dir() {
        return Err(format!("下载文件夹不存在: {save_dir}"));
    }

    // 重名自动编号：file.zip → file(2).zip
    let stem = PathBuf::from(&filename)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "mod".into());
    let ext = PathBuf::from(&filename)
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let mut target = dir.join(&filename);
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem}({n}){ext}"));
        n += 1;
    }
    let target_name = target
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| filename.clone());

    // 注册取消标志
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .insert(task_id.clone(), cancel.clone());

    // 下载（与 market_download 同款进度事件）
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let resp = client
        .get(&download_url)
        .header("User-Agent", "endfield-mod-manager/0.1")
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败：HTTP {}", resp.status()));
    }
    let total = resp.content_length();

    let mut file =
        std::fs::File::create(&target).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut downloaded: u64 = 0;
    let mut resp = resp;
    let mut cancelled = false;
    // timeout 轮询：网络停滞时也能及时响应取消
    loop {
        let chunk = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resp.chunk(),
        )
        .await
        {
            Err(_elapsed) => {
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
                continue;
            }
            Ok(Err(e)) => {
                let err = format!("下载数据失败: {e}");
                drop(file);
                let _ = std::fs::remove_file(&target);
                state
                    .0
                    .lock()
                    .map_err(|e| e.to_string())?
                    .remove(&task_id);
                return Err(err);
            }
            Ok(Ok(None)) => break, // 下载完成
            Ok(Ok(Some(c))) => c,
        };
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        file.write_all(&chunk)
            .map_err(|e| format!("写入文件失败: {e}"))?;
        downloaded += chunk.len() as u64;
        let _ = app.emit(
            "market-dl-progress",
            serde_json::json!({
                "taskId": task_id,
                "filename": target_name,
                "downloaded": downloaded,
                "total": total,
            }),
        );
    }
    drop(file);
    state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&task_id);

    if cancelled {
        let _ = std::fs::remove_file(&target);
        return Err("下载已取消".into());
    }

    if downloaded == 0 {
        let _ = std::fs::remove_file(&target);
        return Err("下载内容为空".into());
    }

    Ok(target.to_string_lossy().to_string())
}

/// 下载图片并保存为 mod 目录的封面（按内容嗅探扩展名，仅支持 png/jpg）
async fn save_mod_cover_image(cover_url: &str, mod_dir: &std::path::Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let resp = client
        .get(cover_url)
        .header("User-Agent", "endfield-mod-manager/0.1")
        .send()
        .await
        .map_err(|e| format!("封面下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("封面下载失败：HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("封面读取失败: {e}"))?;
    if bytes.is_empty() {
        return Err("封面内容为空".into());
    }

    // 按文件头嗅探格式（png / jpg / webp）
    let ext = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        return Err("不支持的封面格式（仅 png/jpg/webp）".into());
    };
    let target = mod_dir.join(format!("_cover.{ext}"));
    std::fs::write(&target, &bytes).map_err(|e| format!("写入封面失败: {e}"))?;
    Ok(())
}

/// 极简 URL 编码（仅处理非 ASCII 与保留字符）
fn urlencoding_lite(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 清理文件/目录名中的非法字符
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

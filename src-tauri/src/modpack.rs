//! 整合包导入：识别「Mods / 角色 / 具体 Mod」三层结构，整体合并进当前 Mods 目录。
//!
//! 语义复刻 QAQM 的三个关键流程：
//!   findIntegratedModsDir     → 找 Mods 层
//!   buildIntegratedCopyPlan   → 逐角色逐条目生成「源 → 目标」计划
//!   movePathWithReplaceSafety → 覆盖前先确保旧内容可还原
//!
//! 与 QAQM 的差异（都是更安全的取舍）：
//!   - 压缩包（zip/7z/rar）直接解压安装，不用先手动解压
//!   - 用户选的文件夹是「复制」（源目录原样保留）；只有我们自己解压的临时目录才「移动」
//!   - 冲突默认跳过，勾选「覆盖」才替换；旧目录进回收站（可还原）
//!   - 保留包里 DISABLED_ 前缀，导入后仍是停用状态（整合包常是别人的 Mods 快照）
//!   - 层级歧义（外层文件夹可能只有一层）交给用户切换：analyze 返回多个候选层级

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

use crate::import_mod::{copy_dir_all, extract_archive, is_archive, move_dir_all, temp_extract_dir};

/// 3DMigoto mod 内部的内容目录名（出现在「角色」位置说明层级判断偏了一层）
const CONTENT_DIRS: &[&str] = &[
    "meshes",
    "textures",
    "shaders",
    "shaderfixes",
    "shaderoverride",
    "disasm",
];

/// 候选层级最多扫描深度
const MAX_DEPTH: usize = 3;

// ===== 前端数据结构 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackEntry {
    /// 条目名（mod 文件夹名或散文件名）
    pub name: String,
    /// "mod"（目录）或 "file"（角色目录下的散文件，如 _cover.png）
    pub kind: String,
    /// 当前 Mods 里是否已有同名 mod（含启用/停用两种形式）
    pub conflict: bool,
    /// 包内该 mod 是否为停用状态（DISABLED_ 前缀）
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackCharacter {
    /// 角色目录名（落盘时沿用磁盘上已有名字，大小写不重复建目录）
    pub name: String,
    /// 当前 Mods 里是否已有该角色
    pub exists: bool,
    pub entries: Vec<PackEntry>,
    pub mod_count: usize,
}

/// 一个可能的「Mods 层」
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackCandidate {
    pub path: String,
    /// 相对用户所选路径的层数（0=所选目录本身）
    pub depth: usize,
    /// 目录名就是 Mods
    pub named_mods: bool,
    pub character_count: usize,
    pub mod_count: usize,
    /// 角色名里有多少个在当前 Mods 目录已存在（层级判定的强信号）
    pub matched: usize,
    /// 给前端显示用，如 "整合包/Mods"
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackInfo {
    /// 用户选择的原始路径
    pub source: String,
    /// 当前采用的 Mods 根目录
    pub mods_root: String,
    /// 若是压缩包，解压出来的临时目录（导入完成/取消后需清理）
    pub temp_dir: Option<String>,
    /// 当前游戏的 Mods 根目录
    pub mods_path: String,
    /// 所有识别到的候选层级（>1 时前端给出切换）
    pub candidates: Vec<PackCandidate>,
    pub characters: Vec<PackCharacter>,
    pub character_count: usize,
    pub mod_count: usize,
    pub conflict_count: usize,
    /// 角色目录下的散文件数量（封面等）
    pub loose_file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackFailure {
    pub name: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModpackResult {
    pub installed: usize,
    pub skipped: usize,
    pub failed: Vec<PackFailure>,
    pub characters: usize,
    pub mods: usize,
    pub replaced: usize,
    /// persist 联动数量（启用=纳入 Bridge 的 mod 数；禁用=还原的 ini 备份数）
    pub persist_touched: usize,
    pub persist_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackProgress {
    pub current: usize,
    pub total: usize,
    pub name: String,
    pub character: String,
    pub installed: usize,
    pub skipped: usize,
    pub failed: usize,
}

// ===== 内部计划结构 =====

struct PlanItem {
    character_dst: String,
    name: String,
    is_dir: bool,
    src: PathBuf,
    dst: PathBuf,
    conflict: bool,
}

/// 一个 Mods 层的内容：角色名 → 该角色下的 mod 目录名
type CharMap = Vec<(String, Vec<String>)>;

// ===== 目录判断工具 =====

/// 3DMigoto 的特殊目录 / 隐藏垃圾，不作为角色或 mod
/// 注意：不能用「以小写 disabled 开头」判断，否则会把 DISABLED_xxx 停用 mod 一起筛掉
fn is_special(name: &str) -> bool {
    let n = name.to_lowercase();
    name.starts_with('.')
        || n == "__macosx"
        || n.starts_with("disabled ") // 「DISABLED - ALL MODS...」
        || n.starts_with("do not modify")
        || n == "crossibclassifier.ini"
        || n.starts_with("qaqm") // 原 QAQM 的缓存目录
}

/// 是否停用状态的名字
fn is_disabled_name(name: &str) -> bool {
    name.len() > 9 && name[..9].eq_ignore_ascii_case("DISABLED_")
}

fn is_content_dir_name(name: &str) -> bool {
    CONTENT_DIRS.contains(&name.to_lowercase().as_str())
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 是否是「一个 mod 目录」：含 mod.ini / Meshes / Textures，
/// 或外层只放预览图、真正的 mod.ini 在下一层（EFMI 常见结构）
fn is_mod_dir(p: &Path) -> bool {
    let direct = |d: &Path| {
        d.join("mod.ini").is_file() || d.join("Meshes").is_dir() || d.join("Textures").is_dir()
    };
    if direct(p) {
        return true;
    }
    if let Ok(entries) = fs::read_dir(p) {
        for e in entries.flatten() {
            let c = e.path();
            if c.is_dir() && direct(&c) {
                return true;
            }
        }
    }
    false
}

/// 若 dir 是 Mods 层，返回「角色 → mod 列表」；否则空
fn read_char_map(dir: &Path) -> CharMap {
    let mut out: CharMap = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    let mut chars: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    chars.sort_by_key(|p| name_of(p));

    for char_dir in chars {
        let cname = name_of(&char_dir);
        if is_special(&cname) || is_content_dir_name(&cname) {
            continue;
        }
        let mut mods: Vec<String> = Vec::new();
        if let Ok(children) = fs::read_dir(&char_dir) {
            let mut items: Vec<PathBuf> = children
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.path())
                .collect();
            items.sort_by_key(|p| name_of(p));
            for m in items {
                let mname = name_of(&m);
                if is_special(&mname) {
                    continue;
                }
                if is_mod_dir(&m) {
                    mods.push(mname);
                }
            }
        }
        if !mods.is_empty() {
            out.push((cname, mods));
        }
    }
    out
}

/// 收集所有可能的 Mods 层（广度优先，最多 MAX_DEPTH 层）
fn collect_candidates(scan_root: &Path, existing: &HashMap<String, String>) -> Vec<PackCandidate> {
    let mut out: Vec<PackCandidate> = Vec::new();
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> =
        std::collections::VecDeque::new();
    queue.push_back((scan_root.to_path_buf(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        let map = read_char_map(&dir);
        if !map.is_empty() {
            let rel = dir
                .strip_prefix(scan_root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let label = if rel.is_empty() {
                let n = name_of(scan_root);
                if n.is_empty() {
                    "所选目录".to_string()
                } else {
                    n
                }
            } else {
                rel
            };
            let matched = map
                .iter()
                .filter(|(c, _)| existing.contains_key(&c.trim().to_lowercase()))
                .count();
            out.push(PackCandidate {
                path: dir.to_string_lossy().to_string(),
                depth,
                named_mods: name_of(&dir).eq_ignore_ascii_case("mods"),
                character_count: map.len(),
                mod_count: map.iter().map(|(_, m)| m.len()).sum(),
                matched,
                label,
            });
        }
        if depth >= MAX_DEPTH {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let n = name_of(&p);
                if is_special(&n) || is_content_dir_name(&n) {
                    continue;
                }
                queue.push_back((p, depth + 1));
            }
        }
    }

    // 排序：名字是 Mods 的优先 → 命中已有角色多的 → 角色多 → Mod 多 → 层级浅（歧义时保守取外层）
    out.sort_by(|a, b| {
        b.named_mods
            .cmp(&a.named_mods)
            .then(b.matched.cmp(&a.matched))
            .then(b.character_count.cmp(&a.character_count))
            .then(b.mod_count.cmp(&a.mod_count))
            .then(a.depth.cmp(&b.depth))
    });
    out
}

/// 读取目标 Mods 目录里已有的角色名：小写名 → 磁盘上的真实名
fn existing_characters(mods_path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(entries) = fs::read_dir(mods_path) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let n = e.file_name().to_string_lossy().to_string();
                map.insert(n.trim().to_lowercase(), n);
            }
        }
    }
    map
}

/// 目标位置是否已存在同名 mod（含启用/停用两种形式）
fn detect_conflict(char_dir: &Path, name: &str) -> bool {
    if char_dir.join(name).exists() {
        return true;
    }
    char_dir.join(alternate_name(name)).exists()
}

/// 同名 mod 的另一种状态名（DISABLED_ 前缀 ↔ 无前缀）
fn alternate_name(name: &str) -> String {
    if is_disabled_name(name) {
        name[9..].to_string()
    } else {
        format!("DISABLED_{name}")
    }
}

/// 根据 Mods 层 + 目标 Mods 目录生成安装计划
fn build_plan(mods_root: &Path, target_mods: &Path) -> Result<Vec<PlanItem>, String> {
    let existing = existing_characters(target_mods);
    let map = read_char_map(mods_root);
    if map.is_empty() {
        return Err("整合包里没有可安装的角色 Mod 内容".into());
    }

    let mut plan = Vec::new();
    for (char_src, mod_names) in &map {
        let char_dst = existing
            .get(&char_src.trim().to_lowercase())
            .cloned()
            .unwrap_or_else(|| char_src.clone());
        let dst_char_dir = target_mods.join(&char_dst);
        let src_char_dir = mods_root.join(char_src);

        // 角色目录下的散文件（封面等）一并带上
        let mut loose: Vec<String> = Vec::new();
        if let Ok(entries) = fs::read_dir(&src_char_dir) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.path())
                .collect();
            files.sort_by_key(|p| name_of(p));
            for f in files {
                let n = name_of(&f);
                if !is_special(&n) {
                    loose.push(n);
                }
            }
        }

        for name in mod_names.iter().cloned().chain(loose) {
            let src = src_char_dir.join(&name);
            if !src.exists() {
                continue;
            }
            let is_dir = src.is_dir();
            let dst = dst_char_dir.join(&name);
            let conflict = detect_conflict(&dst_char_dir, &name);
            plan.push(PlanItem {
                character_dst: char_dst.clone(),
                name,
                is_dir,
                src,
                dst,
                conflict,
            });
        }
    }
    Ok(plan)
}

/// 把计划汇总成前端可展示的 info
fn build_info(
    source: &str,
    mods_path: &str,
    mods_root: &Path,
    temp_dir: Option<String>,
    candidates: Vec<PackCandidate>,
) -> Result<ModpackInfo, String> {
    let target_mods = Path::new(mods_path);
    let plan = build_plan(mods_root, target_mods)?;

    let mut characters: Vec<PackCharacter> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut mod_count = 0usize;
    let mut conflict_count = 0usize;
    let mut loose_file_count = 0usize;
    for item in &plan {
        let idx = match index.get(&item.character_dst) {
            Some(i) => *i,
            None => {
                let i = characters.len();
                index.insert(item.character_dst.clone(), i);
                characters.push(PackCharacter {
                    name: item.character_dst.clone(),
                    exists: target_mods.join(&item.character_dst).is_dir(),
                    entries: Vec::new(),
                    mod_count: 0,
                });
                i
            }
        };
        if item.conflict {
            conflict_count += 1;
        }
        if item.is_dir {
            mod_count += 1;
            characters[idx].mod_count += 1;
        } else {
            loose_file_count += 1;
        }
        characters[idx].entries.push(PackEntry {
            name: item.name.clone(),
            kind: if item.is_dir { "mod".into() } else { "file".into() },
            conflict: item.conflict,
            disabled: is_disabled_name(&item.name),
        });
    }

    Ok(ModpackInfo {
        source: source.to_string(),
        mods_root: mods_root.to_string_lossy().to_string(),
        temp_dir,
        mods_path: mods_path.to_string(),
        candidates,
        character_count: characters.len(),
        characters,
        mod_count,
        conflict_count,
        loose_file_count,
    })
}

/// 分析整合包（不安装，只识别）：文件夹或压缩包都支持
pub fn analyze(mods_path: &str, source: &str) -> Result<ModpackInfo, String> {
    if mods_path.trim().is_empty() {
        return Err("尚未配置 Mods 目录".into());
    }
    let src = Path::new(source);
    if !src.exists() {
        return Err(format!("路径不存在: {source}"));
    }
    let target_mods = Path::new(mods_path);
    if !target_mods.is_dir() {
        return Err(format!("Mods 目录不存在: {mods_path}"));
    }

    let mut temp_dir: Option<PathBuf> = None;
    let scan_root: PathBuf = if src.is_file() {
        if !is_archive(src) {
            return Err("整合包只支持文件夹或 zip / 7z / rar 压缩包".into());
        }
        let tmp = temp_extract_dir("efmi-pack")?;
        match extract_archive(src, &tmp) {
            Ok(()) => {
                temp_dir = Some(tmp.clone());
                tmp
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&tmp);
                return Err(e);
            }
        }
    } else {
        src.to_path_buf()
    };

    let candidates = collect_candidates(&scan_root, &existing_characters(target_mods));
    if candidates.is_empty() {
        if let Some(t) = &temp_dir {
            let _ = fs::remove_dir_all(t);
        }
        return Err(
            "未识别到整合包结构：需要「Mods\\角色\\具体Mod」这样的三层目录。请选择包含 Mods 文件夹的目录，或解压后的整合包文件夹。".into(),
        );
    }

    let best = candidates[0].path.clone();
    let temp_str = temp_dir.as_ref().map(|p| p.to_string_lossy().to_string());
    match build_info(source, mods_path, Path::new(&best), temp_str.clone(), candidates) {
        Ok(info) => Ok(info),
        Err(e) => {
            if let Some(t) = &temp_dir {
                let _ = fs::remove_dir_all(t);
            }
            Err(e)
        }
    }
}

/// 按指定层级重新构建 info（用户在候选层级之间切换时调用）
pub fn inspect(mods_path: &str, mods_root: &str, temp_dir: Option<String>, source: &str) -> Result<ModpackInfo, String> {
    let target_mods = Path::new(mods_path);
    if !target_mods.is_dir() {
        return Err(format!("Mods 目录不存在: {mods_path}"));
    }
    let root = Path::new(mods_root);
    if !root.is_dir() {
        return Err(format!("目录不存在: {mods_root}"));
    }
    // 候选列表按新的根目录重新算（父级/子级切换都能用）
    let scan_base = temp_dir
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| root.starts_with(p))
        .unwrap_or_else(|| root.to_path_buf());
    let candidates = collect_candidates(&scan_base, &existing_characters(target_mods));
    build_info(source, mods_path, root, temp_dir, candidates)
}

/// 把已有目标移到回收站（失败则直接删），用于「覆盖」模式
fn discard_existing(target: &Path) {
    if !target.exists() {
        return;
    }
    if trash::delete(target).is_err() {
        if target.is_dir() {
            let _ = fs::remove_dir_all(target);
        } else {
            let _ = fs::remove_file(target);
        }
    }
}

fn emit_progress(
    app: &AppHandle,
    current: usize,
    total: usize,
    name: &str,
    character: &str,
    installed: usize,
    skipped: usize,
    failed: usize,
) {
    let _ = app.emit(
        "modpack-progress",
        PackProgress {
            current,
            total,
            name: name.to_string(),
            character: character.to_string(),
            installed,
            skipped,
            failed,
        },
    );
}

/// 执行导入。`mods_root` / `temp_dir` 来自 analyze 的结果（避免重复解压）
pub fn import(
    app: &AppHandle,
    mods_path: &str,
    mods_root: &str,
    temp_dir: Option<String>,
    overwrite: bool,
) -> Result<ModpackResult, String> {
    let target_mods = Path::new(mods_path);
    if !target_mods.is_dir() {
        return Err(format!("Mods 目录不存在: {mods_path}"));
    }
    let root = Path::new(mods_root);
    if !root.is_dir() {
        return Err(format!("整合包目录不存在: {mods_root}"));
    }

    // 只在自己的临时解压目录上做「移动」，用户自己的文件夹一律「复制」
    let from_temp = temp_dir
        .as_deref()
        .map(|t| root.starts_with(t))
        .unwrap_or(false);

    let plan = build_plan(root, target_mods)?;
    if plan.is_empty() {
        return Err("整合包里没有可安装的角色 Mod 内容".into());
    }

    let total = plan.len();
    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut replaced = 0usize;
    let mut mods = 0usize;
    let mut failed: Vec<PackFailure> = Vec::new();
    let mut installed_dirs: Vec<PathBuf> = Vec::new();
    let mut characters: Vec<String> = Vec::new();

    emit_progress(app, 0, total, "准备导入…", "", 0, 0, 0);

    for (i, item) in plan.iter().enumerate() {
        let label = format!("{} / {}", item.character_dst, item.name);

        if item.conflict && !overwrite {
            skipped += 1;
            emit_progress(app, i + 1, total, &label, &item.character_dst, installed, skipped, failed.len());
            continue;
        }

        if item.conflict {
            // 覆盖：新旧两种状态都清掉，避免 Mod1 / DISABLED_Mod1 并存
            discard_existing(&item.dst);
            let alt = target_mods
                .join(&item.character_dst)
                .join(alternate_name(&item.name));
            discard_existing(&alt);
            if item.dst.exists() {
                failed.push(PackFailure {
                    name: label.clone(),
                    error: "旧目录被占用，无法替换".into(),
                });
                emit_progress(app, i + 1, total, &label, &item.character_dst, installed, skipped, failed.len());
                continue;
            }
            replaced += 1;
        }

        let result = (|| -> Result<(), String> {
            let parent = item.dst.parent().unwrap_or(target_mods);
            fs::create_dir_all(parent).map_err(|e| format!("创建角色目录失败: {e}"))?;
            if item.is_dir {
                if from_temp {
                    move_dir_all(&item.src, &item.dst)
                } else {
                    copy_dir_all(&item.src, &item.dst)
                }
            } else {
                fs::copy(&item.src, &item.dst)
                    .map(|_| ())
                    .map_err(|e| format!("复制文件失败: {e}"))
            }
        })();

        match result {
            Ok(()) => {
                installed += 1;
                if item.is_dir {
                    mods += 1;
                    installed_dirs.push(item.dst.clone());
                }
                if !characters.contains(&item.character_dst) {
                    characters.push(item.character_dst.clone());
                }
            }
            Err(e) => failed.push(PackFailure {
                name: label.clone(),
                error: e,
            }),
        }
        emit_progress(app, i + 1, total, &label, &item.character_dst, installed, skipped, failed.len());
    }

    // persist 联动：启用则把新 mod 纳入 Bridge，禁用则还原随包带来的 .qaqm-persistbak
    let cfg = crate::config::load_config();
    let persist_enabled = cfg.persist_enabled;
    let mut persist_touched = 0usize;
    if let Some(game_root) = target_mods.parent() {
        if persist_enabled {
            for dir in &installed_dirs {
                if let Ok(n) = crate::persist::migrate_mod_dir(game_root, dir) {
                    if n > 0 {
                        persist_touched += 1;
                    }
                }
            }
            if !installed_dirs.is_empty() {
                let _ = crate::persist::rebuild_active_includes(game_root);
            }
        } else {
            for dir in &installed_dirs {
                persist_touched += crate::persist::restore_bak_in_dir(dir);
            }
        }
    }

    // 清理自己的临时解压目录
    if from_temp {
        if let Some(t) = temp_dir.as_deref() {
            let _ = fs::remove_dir_all(t);
        }
    }

    Ok(ModpackResult {
        installed,
        skipped,
        failed,
        characters: characters.len(),
        mods,
        replaced,
        persist_touched,
        persist_enabled,
    })
}

/// 清理临时解压目录（用户取消分析时调用）
pub fn cleanup(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    let looks_ours = p
        .file_name()
        .map(|n| n.to_string_lossy().starts_with("efmi-pack-"))
        .unwrap_or(false);
    if !looks_ours {
        return Err("拒绝清理非整合包临时目录".into());
    }
    let in_temp = match (std::env::temp_dir().canonicalize(), p.canonicalize()) {
        (Ok(t), Ok(c)) => c.starts_with(t),
        _ => false,
    };
    if !in_temp {
        return Err("拒绝清理临时目录之外的路径".into());
    }
    fs::remove_dir_all(p).map_err(|e| format!("清理临时目录失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, b"x").unwrap();
    }

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("efmi-pack-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// 模拟目标 Mods 里已有的角色
    fn exists_map(names: &[&str]) -> HashMap<String, String> {
        names
            .iter()
            .map(|n| (n.to_lowercase(), n.to_string()))
            .collect()
    }

    #[test]
    fn prefers_named_mods_level() {
        let base = tmp_root("find-mods");
        let mods = base.join("整合包").join("Mods");
        touch(&mods.join("角色A").join("Mod1").join("mod.ini"));
        let candidates = collect_candidates(&base, &HashMap::new());
        assert!(!candidates.is_empty());
        assert_eq!(Path::new(&candidates[0].path), mods);
        assert!(candidates[0].named_mods);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn handles_outer_wrapper_without_mods_dir() {
        let base = tmp_root("wrapper");
        // pack/终末地整合包/角色A/Mod1/mod.ini
        touch(&base.join("终末地整合包").join("角色A").join("Mod1").join("mod.ini"));
        // 目标里已有「角色A」→ 里层层级命中已有角色，应被选中
        let candidates = collect_candidates(&base, &exists_map(&["角色A"]));
        assert!(!candidates.is_empty());
        assert!(candidates[0].path.ends_with("终末地整合包"), "{candidates:?}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn prefers_shallow_for_outer_inner_structure() {
        let base = tmp_root("outer-inner");
        // Mods/角色A/Outer/Inner/mod.ini（外层只放预览图，mod.ini 在下一层）
        touch(
            &base
                .join("Mods")
                .join("角色A")
                .join("Outer")
                .join("Inner")
                .join("mod.ini"),
        );
        let candidates = collect_candidates(&base, &exists_map(&["角色A"]));
        assert_eq!(Path::new(&candidates[0].path), base.join("Mods"), "{candidates:?}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn analyzes_plan_and_conflicts() {
        let base = tmp_root("plan");
        let pack = base.join("pack");
        touch(&pack.join("Mods").join("角色A").join("Mod1").join("mod.ini"));
        touch(&pack.join("Mods").join("角色A").join("Mod2").join("mod.ini"));
        touch(&pack.join("Mods").join("角色A").join("_cover.png"));
        touch(&pack.join("Mods").join("角色B").join("Mod3").join("mod.ini"));
        touch(
            &pack
                .join("Mods")
                .join("角色B")
                .join("DISABLED_Mod4")
                .join("mod.ini"),
        );

        let target = base.join("target");
        touch(&target.join("角色A").join("Mod2").join("mod.ini")); // 同名冲突
        fs::create_dir_all(target.join("角色C")).unwrap();

        let info = analyze(&target.to_string_lossy(), &pack.to_string_lossy()).unwrap();
        assert_eq!(info.character_count, 2);
        assert_eq!(info.mod_count, 4, "含 DISABLED_ 的 mod 也要计入");
        assert_eq!(info.loose_file_count, 1);
        assert_eq!(info.conflict_count, 1);
        assert!(info.temp_dir.is_none());

        let char_a = info.characters.iter().find(|c| c.name == "角色A").unwrap();
        assert!(char_a.exists);
        assert_eq!(char_a.mod_count, 2);
        assert!(char_a.entries.iter().any(|e| e.name == "Mod2" && e.conflict));
        assert!(char_a
            .entries
            .iter()
            .any(|e| e.name == "_cover.png" && e.kind == "file"));
        let char_b = info.characters.iter().find(|c| c.name == "角色B").unwrap();
        assert!(!char_b.exists);
        assert!(char_b.entries.iter().any(|e| e.disabled));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn rejects_single_mod_dir() {
        let base = tmp_root("flat");
        let mod_dir = base.join("Mod1");
        touch(&mod_dir.join("mod.ini"));
        fs::create_dir_all(mod_dir.join("Meshes")).unwrap();
        touch(&mod_dir.join("Meshes").join("a.buf"));
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        assert!(analyze(&target.to_string_lossy(), &mod_dir.to_string_lossy()).is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_refuses_foreign_path() {
        assert!(cleanup("D:/SomeDir").is_err());
        assert!(cleanup("C:/Windows").is_err());
    }
}

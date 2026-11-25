//! Mod 扫描：遍历 Mods 目录，识别「角色 → mod」两级结构。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ModEntry {
    /// mod 显示名（去掉 DISABLED_ 前缀）
    pub name: String,
    /// 所属角色名
    pub character: String,
    /// 磁盘上的实际文件夹名（可能带 DISABLED_ 前缀）
    pub folder_name: String,
    /// 是否启用（不带 DISABLED_ 前缀）
    pub enabled: bool,
    /// 相对 mods_path 的干净路径（用于预设）
    pub rel_path: String,
    /// 是否有封面图
    pub has_cover: bool,
    /// 封面图文件名（相对 mod 目录）
    pub cover: String,
    /// 目录修改时间（Unix 秒），用于排序
    pub modified: u64,
    /// 该 mod 内所有 .ini 声明的游戏资源 hash（3DMigoto hash= 行，用于冲突检测）
    pub hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CharacterGroup {
    pub name: String,
    pub mods: Vec<ModEntry>,
    /// 角色封面图文件名（相对角色目录，如 "_cover.jpg"）
    pub cover: String,
    /// 启用的 mod 数量
    pub enabled_count: usize,
    /// 总 mod 数量
    pub total_count: usize,
    /// 角色名全拼（小写，无空格），用于搜索
    pub pinyin: String,
    /// 角色名拼音首字母（小写），用于搜索
    pub pinyin_initials: String,
}

/// 生成拼音全拼与首字母（非汉字字符原样保留小写）
fn make_pinyin(s: &str) -> (String, String) {
    use pinyin::ToPinyin;
    let mut full = String::new();
    let mut initials = String::new();
    for c in s.chars() {
        if let Some(py) = c.to_pinyin() {
            let plain = py.plain();
            full.push_str(plain);
            if let Some(first) = plain.chars().next() {
                initials.push(first);
            }
        } else if c.is_ascii_alphanumeric() {
            let lower = c.to_ascii_lowercase();
            full.push(lower);
            initials.push(lower);
        }
    }
    (full, initials)
}

/// 去掉 DISABLED_ 前缀，返回干净名
pub fn clean_name(folder: &str) -> String {
    folder
        .strip_prefix("DISABLED_")
        .unwrap_or(folder)
        .to_string()
}

/// 判断一个文件夹名是否是「特殊目录」（不应被当作角色或 mod）
fn is_special_dir(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("disabled") // DISABLED - ALL MODS...
        || n.starts_with("do not modify")
        || n == "crossibclassifier.ini"
}

/// 判断一个文件夹是否是有效的 mod 根（含 mod.ini，或含 Meshes/Textures）
#[allow(dead_code)]
fn is_valid_mod_dir(dir: &Path) -> bool {
    // 直接含 mod.ini
    if dir.join("mod.ini").is_file() {
        return true;
    }
    // 或内层嵌套一层（外层含 tags.json/preview，内层才是 mod）
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let inner = p.join("mod.ini");
                if inner.is_file() {
                    return true;
                }
                // 也接受含 Meshes + Textures 的目录（3DMigoto mod 特征）
                if p.join("Meshes").is_dir() || p.join("Textures").is_dir() {
                    return true;
                }
            }
        }
    }
    false
}

/// 读取 mod 目录下全部 .ini 文件，收集 **Override 段**里的 hash= 声明
/// （[TextureOverride]/[ShaderOverride] 等段的 hash= 表示「替换该游戏资源」；
///  [Resource] 段的 hash= 只是引用共享资源，不算替换声明，否则通用资源会大量误报）
/// 两个启用中的 mod 声明替换同一资源 → 加载冲突（跨目录也算，用于发现装散的 mod）
fn collect_ini_hashes(mod_dir: &Path) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    let mut stack = vec![mod_dir.to_path_buf()];
    let mut ini_visited = 0usize;
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_ini = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("ini"))
                .unwrap_or(false);
            if !is_ini || ini_visited >= 64 {
                continue; // 单 mod 最多解析 64 个 ini，防超大目录拖慢扫描
            }
            ini_visited += 1;
            if let Ok(content) = std::fs::read_to_string(&path) {
                let mut in_override = false;
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with('[') && line.ends_with(']') {
                        // 段切换：Override 段才是「替换声明」（大小写不敏感）
                        let sec = line[1..line.len() - 1].to_lowercase();
                        in_override = sec.contains("override");
                        continue;
                    }
                    if !in_override {
                        continue;
                    }
                    if let Some(rest) = line
                        .strip_prefix("hash")
                        .and_then(|r| r.trim_start().strip_prefix("="))
                    {
                        let val = rest.trim().to_lowercase();
                        if !val.is_empty() {
                            out.insert(val);
                        }
                    }
                }
            }
        }
    }
    out.into_iter().collect()
}

/// 扫描整个 Mods 目录，返回按角色分组的 mod 列表
pub fn scan_mods(mods_path: &str) -> Result<Vec<CharacterGroup>, String> {
    let root = Path::new(mods_path);
    if !root.is_dir() {
        return Err(format!("Mods 目录不存在: {mods_path}"));
    }

    let mut groups: Vec<CharacterGroup> = Vec::new();

    let entries = fs::read_dir(root).map_err(|e| format!("读取 Mods 目录失败: {e}"))?;
    let mut char_dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    // 按名称排序，保证稳定
    char_dirs.sort_by_key(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));

    for char_dir in char_dirs {
        let char_name = match char_dir.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if is_special_dir(&char_name) {
            continue;
        }

        let mut mods: Vec<ModEntry> = Vec::new();
        if let Ok(children) = fs::read_dir(&char_dir) {
            let mut mod_dirs: Vec<PathBuf> = children
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.path())
                .collect();
            mod_dirs.sort_by_key(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));

            for mod_dir in mod_dirs {
                let folder_name = match mod_dir.file_name() {
                    Some(n) => n.to_string_lossy().to_string(),
                    None => continue,
                };
                let name = clean_name(&folder_name);
                let enabled = !folder_name.starts_with("DISABLED_");
                let rel_path = format!("{}/{}", char_name, name);
                let cover = find_cover(&mod_dir);
                let modified = fs::metadata(&mod_dir)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let hashes = collect_ini_hashes(&mod_dir);
                mods.push(ModEntry {
                    name,
                    character: char_name.clone(),
                    folder_name,
                    enabled,
                    rel_path,
                    has_cover: !cover.is_empty(),
                    cover,
                    modified,
                    hashes,
                });
            }
        }

        let enabled_count = mods.iter().filter(|m| m.enabled).count();
        let total_count = mods.len();
        let cover = find_cover(&char_dir);
        let (pinyin, pinyin_initials) = make_pinyin(&char_name);

        groups.push(CharacterGroup {
            name: char_name,
            mods,
            cover,
            enabled_count,
            total_count,
            pinyin,
            pinyin_initials,
        });
    }

    Ok(groups)
}

/// 查找目录下的封面图文件名（_cover.png 优先——用户自定义封面，其次 _cover.*、preview.*）
fn find_cover(dir: &Path) -> String {
    let names = ["_cover.png", "_cover.jpg", "_cover.webp", "preview.png", "preview.jpg", "cover.png", "cover.jpg", "cover.webp"];
    for n in names {
        if dir.join(n).is_file() {
            return n.to_string();
        }
    }
    // 兜底：找第一个 .jpg/.png/.webp
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if lower.ends_with(".jpg") || lower.ends_with(".png") || lower.ends_with(".webp") {
                return name;
            }
        }
    }
    String::new()
}

/// 校验某个角色文件夹里的具体 mod 目录是否有效
#[allow(dead_code)]
pub fn is_valid_mod_folder(dir: &Path) -> bool {
    is_valid_mod_dir(dir)
}

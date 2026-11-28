//! 独立的 PAK 文件管理，不负责游戏加载或签名处理。
use serde::Serialize;
use std::{collections::{BTreeMap, BTreeSet}, fs, path::{Component, Path, PathBuf}};

#[derive(Serialize)]
pub struct PakEntry { pub character: String, pub name: String, pub enabled: bool, pub groups: usize, pub error: Option<String>, pub conflict: bool, pub modified: u64, pub cover: String }

static PAK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 验证异环根目录并推算 PAK 生效目录；不创建任何文件夹。
#[tauri::command]
pub fn pak_active_path(game_root: String) -> Result<String, String> {
    let root = fs::canonicalize(&game_root).map_err(|e| format!("无法访问游戏根目录: {e}"))?;
    if !root.is_dir() { return Err("请选择游戏根文件夹".into()); }
    let mut has_launcher = false;
    for entry in fs::read_dir(&root).map_err(|e| format!("无法读取游戏根目录: {e}"))? {
        let entry = entry.map_err(|e| format!("读取游戏根目录失败: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.contains("launcher") && name.ends_with(".exe") && entry.file_type().map_err(|e| e.to_string())?.is_file() {
            has_launcher = true;
        }
    }
    if !has_launcher { return Err("游戏根目录中未找到名称包含 Launcher 的 exe 文件".into()); }
    let paks = root.join("Client").join("WindowsNoEditor").join("HT").join("Content").join("Paks");
    if !paks.is_dir() { return Err(format!("游戏根目录中未找到 Paks 目录: {}", paks.display())); }
    Ok(paks.join("MOD").to_string_lossy().into_owned())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 { return true; }
    }
    false
}

fn segment(value: &str) -> Result<&str, String> {
    let mut parts = Path::new(value).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(part)), None) if !value.trim().is_empty() && !value.ends_with([' ', '.']) && part.to_string_lossy() == value && !value.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) => Ok(value),
        _ => Err("名称必须是单个合法目录名".into()),
    }
}

fn roots(active: &str, disabled: &str) -> Result<(PathBuf, PathBuf), String> {
    let a = fs::canonicalize(active).map_err(|e| format!("无法访问生效目录: {e}"))?;
    let b = fs::canonicalize(disabled).map_err(|e| format!("无法访问停用目录: {e}"))?;
    if !a.is_dir() || !b.is_dir() || a.starts_with(&b) || b.starts_with(&a) { return Err("生效目录和停用目录必须是互不包含的现有目录".into()); }
    Ok((a, b))
}

fn groups(dir: &Path) -> Result<usize, String> {
    fn visit(dir: &Path, found: &mut BTreeMap<(PathBuf, String), BTreeSet<String>>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if is_link(&fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?) { return Err(format!("不支持链接: {}", entry.path().display())); }
            if ty.is_dir() { visit(&entry.path(), found)?; }
            else if ty.is_file() {
                let p = entry.path();
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                if ["pak", "ucas", "utoc"].contains(&ext.as_str()) {
                    let stem = p.file_stem().and_then(|x| x.to_str()).ok_or("文件名不是有效文本")?.to_lowercase();
                    let parent = p.parent().ok_or("文件路径无父目录")?.to_path_buf();
                    if !found.entry((parent, stem)).or_default().insert(ext) { return Err("存在大小写冲突的 PAK 文件".into()); }
                }
            }
        }
        Ok(())
    }
    let mut found = BTreeMap::new();
    visit(dir, &mut found)?;
    if found.is_empty() || found.values().any(|exts| exts.len() != 3) { return Err("未找到完整 PAK 三件套，或存在不完整的文件组".into()); }
    Ok(found.len())
}

fn safe_child(root: &Path, character: &str, name: &str) -> Result<PathBuf, String> {
    let character = segment(character)?; let name = segment(name)?;
    let char_path = root.join(character);
    if char_path.exists() {
        let canonical = fs::canonicalize(&char_path).map_err(|e| e.to_string())?;
        if !canonical.starts_with(root) || is_link(&fs::symlink_metadata(&char_path).map_err(|e| e.to_string())?) { return Err("角色目录包含不安全的链接".into()); }
    }
    Ok(char_path.join(name))
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir(target).map_err(|e| format!("创建目标目录失败: {e}"))?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?; let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = target.join(entry.file_name());
        if is_link(&fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?) { return Err("不支持导入链接或重解析点".into()); }
        if ty.is_dir() { copy_tree(&entry.path(), &to)?; }
        else if ty.is_file() { fs::copy(entry.path(), to).map_err(|e| format!("复制失败: {e}"))?; }
        else { return Err("不支持特殊文件".into()); }
    }
    Ok(())
}

/// 在 Mod 目录及其子目录中查找预览图，返回相对 Mod 目录的路径（`/` 分隔，找不到为空）。
/// 优先 `preview.*`，其次 `_cover.*`、`cover.*`；都没有时回退最浅层的第一张图片。跳过链接，最深 8 层。
fn find_pak_cover(dir: &Path) -> String {
    const PREFERRED: [&str; 9] = ["preview.png", "preview.jpg", "preview.webp", "_cover.png", "_cover.jpg", "_cover.webp", "cover.png", "cover.jpg", "cover.webp"];
    const MAX_DEPTH: usize = 8;
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((dir.to_path_buf(), 0usize, String::new()));
    let mut images: Vec<(usize, String)> = Vec::new();
    while let Some((current, depth, relative)) = queue.pop_front() {
        if depth >= MAX_DEPTH { continue; }
        let Ok(entries) = fs::read_dir(&current) else { continue };
        let mut subdirs: Vec<(PathBuf, usize, String)> = Vec::new();
        for entry in entries.flatten() {
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else { continue };
            if is_link(&metadata) { continue; }
            let name = match entry.file_name().into_string() { Ok(name) => name, Err(_) => continue };
            let child = if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
            if metadata.is_dir() { subdirs.push((entry.path(), depth + 1, child)); }
            else if metadata.is_file() && ["png", "jpg", "jpeg", "webp"].iter().any(|ext| name.to_ascii_lowercase().ends_with(&format!(".{ext}"))) {
                images.push((depth, child));
            }
        }
        queue.extend(subdirs);
    }
    images.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase())));
    for name in PREFERRED {
        if let Some((_, found)) = images.iter().find(|(_, candidate)| candidate.rsplit('/').next().is_some_and(|file| file.eq_ignore_ascii_case(name))) {
            return found.clone();
        }
    }
    images.first().map(|(_, found)| found.clone()).unwrap_or_default()
}

/// 为市场安装的 PAK 保存封面；保留压缩包中已有的预览图片。
pub(crate) fn save_market_cover(active: &str, disabled: &str, character: &str, name: &str, bytes: &[u8], ext: &str) -> Result<(), String> {
    use std::io::Write;
    if !["png", "jpg", "webp"].contains(&ext) { return Err("不支持的封面格式".into()); }
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (a, b) = roots(active, disabled)?;
    let dir = safe_child(&a, character, name)?;
    let other = safe_child(&b, character, name)?;
    if other.exists() { return Err("两个目录存在同名 Mod".into()); }
    let metadata = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || is_link(&metadata) { return Err("Mod 不是普通目录".into()); }
    if !find_pak_cover(&dir).is_empty() { return Ok(()); }
    let target = dir.join(format!("_cover.{ext}"));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(target).map_err(|e| format!("创建封面失败: {e}"))?;
    file.write_all(bytes).map_err(|e| format!("写入封面失败: {e}"))
}

#[tauri::command]
pub fn pak_scan(active: String, disabled: String) -> Result<Vec<PakEntry>, String> {
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (a, b) = roots(&active, &disabled)?; let mut output: Vec<PakEntry> = Vec::new();
    for (root, enabled) in [(&a, true), (&b, false)] {
        for c in fs::read_dir(root).map_err(|e| e.to_string())? {
            let c = c.map_err(|e| e.to_string())?;
            let meta = fs::symlink_metadata(c.path()).map_err(|e| e.to_string())?;
            if is_link(&meta) { return Err(format!("角色目录包含链接或重解析点: {}", c.path().display())); }
            if !meta.is_dir() { continue; }
            let character = c.file_name().to_string_lossy().to_string();
            for m in fs::read_dir(c.path()).map_err(|e| e.to_string())? {
                let m = m.map_err(|e| e.to_string())?;
                let meta = fs::symlink_metadata(m.path()).map_err(|e| e.to_string())?;
                if !meta.is_dir() && !is_link(&meta) { continue; }
                let result = if is_link(&meta) { Err("不支持链接或重解析点".to_string()) } else { groups(&m.path()) };
                let (count, error) = match result { Ok(count) => (count, None), Err(error) => (0, Some(error)) };
                let modified = meta.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or_default();
                let cover = if is_link(&meta) { String::new() } else { find_pak_cover(&m.path()) };
                output.push(PakEntry { character: character.clone(), name: m.file_name().to_string_lossy().to_string(), enabled, groups: count, error, conflict: false, modified, cover });
            }
        }
    }
    let mut counts = BTreeMap::new();
    for e in &output { *counts.entry((e.character.to_lowercase(), e.name.to_lowercase())).or_insert(0usize) += 1; }
    for e in &mut output { e.conflict = counts.get(&(e.character.to_lowercase(), e.name.to_lowercase())).copied().unwrap_or(0) > 1; }
    output.sort_by(|a,b| (&a.character, &a.name, a.enabled).cmp(&(&b.character, &b.name, b.enabled))); Ok(output)
}

fn extract_pak_zip(source: &Path, target: &Path) -> Result<(), String> {
    use std::io::{Read, Write};
    const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
    const MAX_FILES: usize = 20_000;
    let file = fs::File::open(source).map_err(|e| format!("打开 ZIP 失败: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("读取 ZIP 失败: {e}"))?;
    if zip.len() > MAX_FILES { return Err("ZIP 文件数量超过限制".into()); }
    fs::create_dir(target).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
        let path = entry.enclosed_name().ok_or("ZIP 包含不安全路径")?;
        for part in path.components() {
            match part { Component::Normal(s) => { segment(s.to_str().ok_or("ZIP 文件名不是有效文本")?)?; }, _ => return Err("ZIP 包含不安全路径".into()) }
        }
        if entry.unix_mode().is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000 && mode & 0o170000 != 0o040000) { return Err("ZIP 包含链接或特殊文件".into()); }
        total = total.checked_add(entry.size()).ok_or("ZIP 解压大小超限")?;
        if total > MAX_TOTAL { return Err("ZIP 解压大小超过 8 GiB 限制".into()); }
        let dest = target.join(path);
        if entry.is_dir() { fs::create_dir_all(&dest).map_err(|e| e.to_string())?; continue; }
        fs::create_dir_all(dest.parent().ok_or("ZIP 路径无效")?).map_err(|e| e.to_string())?;
        let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&dest).map_err(|e| format!("ZIP 文件冲突或无法写入: {e}"))?;
        let expected = entry.size();
        let copied = std::io::copy(&mut entry.by_ref().take(expected.saturating_add(1)), &mut output).map_err(|e| format!("解压 ZIP 失败: {e}"))?;
        if copied != expected { return Err("ZIP 解压大小不一致".into()); }
        output.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 校验解压内容：递归找到包含 PAK 文件的目录（取 PAK 最多的），要求其中的 PAK 均有完整三件套。
/// 嵌套结构原样保留，不移动文件。
fn validate_pak_tree(root: &Path) -> Result<(), String> {
    let mut best: Option<(usize, PathBuf)> = None;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut pak_count = 0usize;
        let mut subdirs = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if is_link(&meta) { return Err(format!("压缩包内包含链接: {}", entry.path().display())); }
            if meta.is_dir() { subdirs.push(entry.path()); }
            else if meta.is_file() && entry.path().extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("pak")).unwrap_or(false) {
                pak_count += 1;
            }
        }
        if pak_count > 0 && best.as_ref().map_or(true, |(count, _)| pak_count > *count) {
            best = Some((pak_count, dir.clone()));
        }
        stack.extend(subdirs);
    }
    let best_dir = best.map(|(_, dir)| dir).ok_or("压缩包内未找到 PAK 文件")?;
    groups(&best_dir)?;
    Ok(())
}

fn extract_pak_archive(source: &Path, target: &Path) -> Result<(), String> {
    // ZIP 走本模块的严格通道（大小/数量上限、拒绝特殊条目、防覆盖）；7z/RAR 复用普通游戏导入的解压实现
    let is_zip = source.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false);
    if is_zip { extract_pak_zip(source, target) } else {
        fs::create_dir(target).map_err(|e| format!("创建解压目录失败: {e}"))?;
        crate::import_mod::extract_archive(source, target)
    }
}

#[tauri::command]
pub fn pak_import(active: String, disabled: String, source: String, character: String, name: String) -> Result<(), String> {
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (a,b) = roots(&active, &disabled)?;
    let source_meta = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
    if is_link(&source_meta) { return Err("导入源不支持链接或重解析点".into()); }
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if source.starts_with(&a) || source.starts_with(&b) || a.starts_with(&source) || b.starts_with(&source) { return Err("请选择管理目录之外的 Mod 文件夹或压缩包".into()); }
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let is_archive = source.is_file() && ["zip", "7z", "rar"].contains(&ext.as_str());
    if !source.is_dir() && !is_archive { return Err("仅支持已解压的文件夹或 zip / 7z / rar 压缩包".into()); }
    if !is_archive { groups(&source)?; }
    let target = safe_child(&a, &character, &name)?; let other = safe_child(&b, &character, &name)?;
    if target.exists() || other.exists() { return Err("已存在同名 Mod，未覆盖".into()); }
    let parent = target.parent().ok_or("无效目标目录")?; fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new().prefix(".pak-import-").tempdir_in(parent).map_err(|e| e.to_string())?;
    let copied = staging.path().join("content");
    if is_archive {
        extract_pak_archive(&source, &copied)?;
        validate_pak_tree(&copied)?;
    } else { copy_tree(&source, &copied)?; }
    groups(&copied)?;
    if target.exists() || other.exists() { return Err("目标已存在，未覆盖".into()); }
    fs::rename(&copied, &target).map_err(|e| format!("完成导入失败: {e}"))?; Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_cover_uses_active_root_only() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let a = tmp.path().join("active"); let b = tmp.path().join("disabled");
        fs::create_dir(&a)?; fs::create_dir(&b)?;
        let image = tmp.path().join("image.png"); fs::write(&image, b"test image")?;
        let active = a.to_string_lossy().into_owned(); let disabled = b.to_string_lossy().into_owned();
        pak_set_character_cover(active.clone(), disabled.clone(), "role".into(), image.to_string_lossy().into_owned())?;
        assert_eq!(fs::read(a.join("role/_cover.png"))?, b"test image");
        assert!(!b.join("role").exists());
        assert_eq!(pak_character_covers(active, disabled)?, vec!["role"]);
        Ok(())
    }

    #[test]
    fn character_config_key_conflicts_never_overwrite() -> Result<(), String> {
        let mut map = std::collections::HashMap::from([("old/mod".to_string(), 1), ("new/mod".to_string(), 2)]);
        assert!(rename_keys(&mut map, "old", "new").is_err());
        assert_eq!(map.get("old/mod"), Some(&1));
        assert_eq!(map.get("new/mod"), Some(&2));
        map.remove("new/mod");
        rename_keys(&mut map, "old", "new")?;
        assert_eq!(map.get("new/mod"), Some(&1));
        Ok(())
    }

    #[test]
    fn preset_restores_recorded_entries_and_stops_on_conflict() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let a = tmp.path().join("active");
        let b = tmp.path().join("disabled");
        for (root, name) in [(&b, "move"), (&a, "conflict"), (&b, "conflict"), (&b, "later"), (&a, "unlisted")] {
            let path = root.join("role").join(name);
            fs::create_dir_all(&path)?;
            for ext in ["pak", "ucas", "utoc"] { fs::write(path.join(format!("mod.{ext}")), b"content")?; }
        }
        let entry = |name: &str| crate::config::PakPresetEntry { character: "role".into(), name: name.into(), enabled: true };
        let entries = [entry("missing"), entry("move"), entry("conflict"), entry("later")];
        let a_root = fs::canonicalize(&a)?;
        let b_root = fs::canonicalize(&b)?;
        let check = check_pak_entries(&a_root, &b_root, &entries)?;
        assert_eq!(check.missing, vec!["role/missing"]);
        assert_eq!(check.conflicts, vec!["role/conflict"]);
        let result = apply_pak_entries(&a_root, &b_root, &entries, &[]);
        assert_eq!(result.moved, 1);
        assert_eq!(result.missing, vec!["role/missing"]);
        assert!(result.error.as_deref().is_some_and(|e| e.contains("conflict")));
        assert!(a.join("role/move").is_dir());
        assert!(b.join("role/later").is_dir());
        assert!(a.join("role/unlisted").is_dir());
        let confirmed = apply_pak_entries(&a_root, &b_root, &entries, &check.conflicts);
        assert!(confirmed.error.is_none());
        assert_eq!(confirmed.moved, 1);
        assert!(a.join("role/later").is_dir());
        assert!(a.join("role/conflict").is_dir() && b.join("role/conflict").is_dir());
        Ok(())
    }

    #[test]
    fn preset_stops_on_invalid_mod_without_moving_later_entries() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let a = tmp.path().join("active");
        let b = tmp.path().join("disabled");
        fs::create_dir_all(a.join("role/broken"))?;
        fs::create_dir_all(a.join("role/later"))?;
        fs::create_dir_all(&b)?;
        let entry = |name: &str| crate::config::PakPresetEntry { character: "role".into(), name: name.into(), enabled: false };
        let result = apply_pak_entries(&fs::canonicalize(&a)?, &fs::canonicalize(&b)?, &[entry("broken"), entry("later")], &[]);
        assert_eq!(result.moved, 0);
        assert!(result.error.is_some());
        assert!(a.join("role/later").is_dir());
        Ok(())
    }
    #[test]
    fn external_archive_format_smoke_test() -> Result<(), Box<dyn std::error::Error>> {
        let Ok(base) = std::env::var("PAK_FORMAT_TEST_DIR") else { return Ok(()); };
        let tmp = tempfile::tempdir()?;
        let active = tmp.path().join("active");
        let disabled = tmp.path().join("disabled");
        fs::create_dir(&active)?;
        fs::create_dir(&disabled)?;
        for ext in ["zip", "7z"] {
            let archive = Path::new(&base).join(format!("sample.{ext}"));
            pak_import(active.to_string_lossy().into_owned(), disabled.to_string_lossy().into_owned(), archive.to_string_lossy().into_owned(), "角色".into(), ext.into())?;
            for part in ["pak", "ucas", "utoc"] {
                assert_eq!(fs::read(active.join(format!("角色/{ext}/wrapper/test.{part}")))?, format!("sample-{part}").as_bytes());
            }
        }
        if let Ok(rar) = std::env::var("PAK_FORMAT_TEST_RAR") {
            let extracted = tmp.path().join("rar-extracted");
            extract_pak_archive(Path::new(&rar), &extracted)?;
            assert_eq!(fs::read(extracted.join(".gitignore"))?, b"target\nCargo.lock\n");
            assert!(pak_import(active.to_string_lossy().into_owned(), disabled.to_string_lossy().into_owned(), rar, "角色".into(), "rar".into()).is_err());
            assert!(!active.join("角色/rar").exists());
        }
        Ok(())
    }
    #[test]
    fn pak_archive_import_peels_wrappers() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        let tmp = tempfile::tempdir()?;
        let active = tmp.path().join("active"); let disabled = tmp.path().join("disabled");
        fs::create_dir(&active)?; fs::create_dir(&disabled)?;
        let archive = tmp.path().join("mod.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&archive)?);
        for ext in ["pak", "ucas", "utoc"] {
            writer.start_file(format!("wrapper/inner/test.{ext}"), zip::write::SimpleFileOptions::default())?;
            writer.write_all(b"test")?;
        }
        writer.finish()?;
        pak_import(active.to_string_lossy().into_owned(), disabled.to_string_lossy().into_owned(), archive.to_string_lossy().into_owned(), "角色".into(), "模组".into())?;
        assert!(active.join("角色/模组/wrapper/inner/test.pak").exists(), "嵌套结构应原样保留");        // 无 PAK 的压缩包应被拒绝
        let empty = tmp.path().join("empty.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&empty)?);
        writer.start_file("readme.txt", zip::write::SimpleFileOptions::default())?;
        writer.write_all(b"nothing")?;
        writer.finish()?;
        assert!(pak_import(active.to_string_lossy().into_owned(), disabled.to_string_lossy().into_owned(), empty.to_string_lossy().into_owned(), "角色".into(), "其他".into()).is_err());
        Ok(())
    }
    #[test]
    fn cross_volume_copy_preserves_content_and_conflicts() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let src = tmp.path().join("source");
        let dst = tmp.path().join("target");
        fs::create_dir_all(src.join("nested"))?;
        for ext in ["pak", "ucas", "utoc"] { fs::write(src.join(format!("nested/test.{ext}")), format!("content-{ext}"))?; }
        fs::write(src.join("preview.png"), b"preview")?;
        fs::create_dir(&dst)?;
        fs::write(dst.join("keep"), b"existing")?;
        assert!(move_cross_volume(&src, &dst).is_err());
        assert_eq!(fs::read(dst.join("keep"))?, b"existing");
        assert!(src.join("nested/test.pak").exists());
        let copied = tmp.path().join("copied");
        copy_tree(&src, &copied)?;
        verify_copied_tree(&src, &copied)?;
        fs::write(copied.join("preview.png"), b"different")?;
        assert!(verify_copied_tree(&src, &copied).is_err());
        assert!(src.join("nested/test.pak").exists());
        let destination = tmp.path().join("success");
        move_cross_volume(&src, &destination)?;
        assert!(!src.exists());
        assert_eq!(fs::read(destination.join("nested/test.pak"))?, b"content-pak");
        assert_eq!(fs::read(destination.join("preview.png"))?, b"preview");
        Ok(())
    }
    #[test]
    fn market_cover_is_detected_and_preserves_existing_image() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let active = tmp.path().join("active");
        let disabled = tmp.path().join("disabled");
        let dir = active.join("角色/模组");
        fs::create_dir_all(&dir)?;
        fs::create_dir(&disabled)?;
        for ext in ["pak", "ucas", "utoc"] { fs::write(dir.join(format!("test.{ext}")), b"test")?; }
        let a = active.to_string_lossy().into_owned();
        let b = disabled.to_string_lossy().into_owned();
        save_market_cover(&a, &b, "角色", "模组", b"cover", "jpg")?;
        assert_eq!(pak_scan(a.clone(), b.clone())?[0].cover, "_cover.jpg");
        fs::create_dir(dir.join("nested"))?;
        fs::write(dir.join("nested/preview.png"), b"existing")?;
        save_market_cover(&a, &b, "角色", "模组", b"replacement", "jpg")?;
        assert_eq!(fs::read(dir.join("_cover.jpg"))?, b"cover");
        assert_eq!(pak_scan(a, b)?[0].cover, "nested/preview.png");
        Ok(())
    }
    #[test]
    fn cover_save_scan_replace_toggle_and_conflict() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let active = tmp.path().join("active");
        let disabled = tmp.path().join("disabled");
        fs::create_dir_all(active.join("角色/模组"))?;
        fs::create_dir(&disabled)?;
        for ext in ["pak", "ucas", "utoc"] { fs::write(active.join(format!("角色/模组/test.{ext}")), b"test")?; }
        fs::write(active.join("角色/模组/preview.jpg"), b"old")?;
        fs::create_dir(active.join("角色/模组/内层"))?;
        fs::write(active.join("角色/模组/内层/preview.png"), b"nested")?;
        let a = active.to_string_lossy().into_owned();
        let d = disabled.to_string_lossy().into_owned();
        let save = |enabled, data: &str| pak_save_cover(a.clone(), d.clone(), "角色".into(), "模组".into(), enabled, data.into());
        assert_eq!(pak_scan(a.clone(), d.clone())?[0].cover, "内层/preview.png");
        assert!(save(true, "bm90IGFuIGltYWdl").is_err());
        save(true, "iVBORw0KGgo=")?;
        save(true, "iVBORw0KGgoBAg==")?;
        assert_eq!(fs::read(active.join("角色/模组/preview.png"))?, b"\x89PNG\r\n\x1a\n\x01\x02");
        assert_eq!(pak_scan(a.clone(), d.clone())?[0].cover, "preview.png");
        pak_toggle(a.clone(), d.clone(), "角色".into(), "模组".into(), false)?;
        let entry = pak_scan(a.clone(), d.clone())?.remove(0);
        assert!(!entry.enabled);
        assert_eq!(entry.cover, "preview.png");
        save(false, "iVBORw0KGgo=")?;
        fs::create_dir_all(active.join("角色/模组"))?;
        assert!(save(false, "iVBORw0KGgoBAg==").is_err());
        assert_eq!(fs::read(disabled.join("角色/模组/preview.png"))?, b"\x89PNG\r\n\x1a\n");
        Ok(())
    }
    #[test]
    fn guess_matches_chinese_english_and_pinyin() {
        eprintln!("pinyin debug: {:?}", make_pinyin("薄荷"));
        let candidates = vec!["Mint".to_string(), "Iroi".to_string(), "薄荷".to_string(), "自定义角色A".to_string()];
        assert_eq!(guess_character(&candidates, "Mint_Dress"), "Mint");
        assert_eq!(guess_character(&candidates, "Iroi_Skin"), "Iroi");
        assert_eq!(guess_character(&candidates, "baohe_skin"), "薄荷");
        assert_eq!(guess_character(&candidates, "MyMod_自定义角色A"), "自定义角色A");
        assert_eq!(guess_character(&candidates, "Unknown"), "");
        let initials = vec!["女主".to_string()];
        assert_eq!(guess_character(&initials, "nz_Casual"), "女主");
        assert_eq!(guess_character(&initials, "hello"), "");
    }
    #[test]
    fn delete_rejects_conflict_and_invalid_paths() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let a = tmp.path().join("active"); let b = tmp.path().join("disabled");
        fs::create_dir_all(a.join("角色/模组"))?; fs::create_dir_all(b.join("角色/模组"))?;
        fs::write(a.join("角色/模组/keep"), b"keep")?;
        let active = a.to_string_lossy().into_owned(); let disabled = b.to_string_lossy().into_owned();
        assert!(pak_delete(active.clone(), disabled.clone(), "角色".into(), "模组".into(), true).is_err());
        assert!(pak_delete(active.clone(), disabled, "../角色".into(), "模组".into(), true).is_err());
        assert!(a.join("角色/模组/keep").exists());
        assert!(b.join("角色/模组").exists());
        Ok(())
    }
    #[test]
    fn zip_import_and_reject_unsafe_entries() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        let tmp = tempfile::tempdir()?;
        let active = tmp.path().join("active"); let disabled = tmp.path().join("disabled");
        fs::create_dir(&active)?; fs::create_dir(&disabled)?;
        let archive = tmp.path().join("mod.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&archive)?);
        for ext in ["pak", "ucas", "utoc"] { writer.start_file(format!("nested/test.{ext}"), zip::write::SimpleFileOptions::default())?; writer.write_all(b"test")?; }
        writer.finish()?;
        pak_import(active.to_string_lossy().into_owned(), disabled.to_string_lossy().into_owned(), archive.to_string_lossy().into_owned(), "角色".into(), "Mod".into())?;
        assert!(active.join("角色/Mod/nested/test.pak").exists());
        let bad = tmp.path().join("bad.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&bad)?);
        writer.start_file("../escape.pak", zip::write::SimpleFileOptions::default())?; writer.write_all(b"test")?; writer.finish()?;
        assert!(pak_import(active.to_string_lossy().into_owned(), disabled.to_string_lossy().into_owned(), bad.to_string_lossy().into_owned(), "角色".into(), "坏包".into()).is_err());
        assert!(!tmp.path().join("escape.pak").exists());
        Ok(())
    }
    #[test]
    fn import_toggle_and_reject_conflicts() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let active = tmp.path().join("active"); let disabled = tmp.path().join("disabled"); let source = tmp.path().join("source");
        fs::create_dir(&active)?; fs::create_dir(&disabled)?; fs::create_dir(&source)?;
        for ext in ["pak", "ucas", "utoc"] { fs::write(source.join(format!("test.{ext}")), ext)?; }
        let a = active.to_string_lossy().to_string(); let d = disabled.to_string_lossy().to_string(); let s = source.to_string_lossy().to_string();
        pak_import(a.clone(), d.clone(), s.clone(), "角色".into(), "模组".into())?;
        assert_eq!(pak_scan(a.clone(), d.clone())?.len(), 1);
        assert!(pak_import(a.clone(), d.clone(), s, "角色".into(), "模组".into()).is_err());
        pak_toggle(a.clone(), d.clone(), "角色".into(), "模组".into(), false)?;
        assert!(disabled.join("角色/模组/test.pak").exists());
        pak_toggle(a, d, "角色".into(), "模组".into(), true)?;
        assert!(active.join("角色/模组/test.pak").exists());
        Ok(())
    }
    #[test]
    fn derives_active_path_without_creating_it() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let paks = tmp.path().join("Client/WindowsNoEditor/HT/Content/Paks");
        fs::create_dir_all(&paks)?;
        assert!(pak_active_path(tmp.path().to_string_lossy().into_owned()).is_err());
        let exe = tmp.path().join("NTEGlobalLauncher.exe"); fs::write(&exe, "test")?;
        let derived = pak_active_path(tmp.path().to_string_lossy().into_owned())?;
        assert_eq!(PathBuf::from(derived), fs::canonicalize(&paks)?.join("MOD"));
        assert!(!paks.join("MOD").exists());
        fs::remove_file(&exe)?;
        fs::write(tmp.path().join("NTELAUNCHER.EXE"), "test")?;
        assert!(pak_active_path(tmp.path().to_string_lossy().into_owned()).is_ok());
        assert!(pak_active_path(tmp.path().join("NTELAUNCHER.EXE").to_string_lossy().into_owned()).is_err());
        fs::remove_dir_all(&paks)?;
        assert!(pak_active_path(tmp.path().to_string_lossy().into_owned()).is_err());
        Ok(())
    }
    #[test]
    fn scan_reports_conflicts_and_incomplete_mods() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let a = tmp.path().join("active"); let b = tmp.path().join("disabled");
        for root in [&a, &b] {
            let folder = root.join("角色").join("重复"); fs::create_dir_all(&folder)?;
            for ext in ["pak", "ucas", "utoc"] { fs::write(folder.join(format!("test.{ext}")), ext)?; }
        }
        let incomplete = a.join("角色").join("不完整"); fs::create_dir_all(&incomplete)?; fs::write(incomplete.join("test.pak"), "x")?;
        let entries = pak_scan(a.to_string_lossy().to_string(), b.to_string_lossy().to_string())?;
        assert_eq!(entries.iter().filter(|e| e.conflict).count(), 2);
        assert!(entries.iter().any(|e| e.name == "不完整" && e.error.is_some()));
        Ok(())
    }
    #[test]
    fn rejects_incomplete_and_unsafe_paths() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?; fs::write(tmp.path().join("test.pak"), "data")?;
        assert!(groups(tmp.path()).is_err());
        assert!(segment("../bad").is_err()); assert!(segment("C:bad").is_err());
        Ok(())
    }
}

/// 检查异环 Mod 在指定目录中存在，且两侧不存在同名冲突。
pub fn validate_pak_entry(active: &str, disabled: &str, character: &str, name: &str, enabled: bool) -> Result<(), String> {
    let _guard = PAK_LOCK.lock().map_err(|e| format!("管理锁不可用: {e}"))?;
    let (a, b) = roots(active, disabled)?;
    let (root, other) = if enabled { (&a, &b) } else { (&b, &a) };
    let target = safe_child(root, character, name)?;
    let other = safe_child(other, character, name)?;
    match fs::symlink_metadata(other) {
        Ok(_) => return Err("两个目录存在同名 Mod，请先处理冲突".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(format!("无法检查另一目录: {e}")),
    }
    let meta = fs::symlink_metadata(target).map_err(|e| format!("读取 Mod 目录失败: {e}"))?;
    if !meta.is_dir() || is_link(&meta) { return Err("Mod 不是普通目录".into()); }
    Ok(())
}

/// 保存异环 Mod 预览图，按当前生效状态选择目录。
#[tauri::command]
pub fn pak_save_cover(active: String, disabled: String, character: String, name: String, enabled: bool, base64_data: String) -> Result<(), String> {
    use std::io::Write;
    const MAX_BYTES: usize = 20 * 1024 * 1024;
    if base64_data.len() > MAX_BYTES.div_ceil(3) * 4 { return Err("图片超过 20 MiB".into()); }
    let bytes = crate::base64_decode(&base64_data)?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES { return Err("图片为空或超过 20 MiB".into()); }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Err("预览图必须是 PNG 格式".into()); }
    validate_pak_entry(&active, &disabled, &character, &name, enabled)?;
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (a, b) = roots(&active, &disabled)?;
    let dir = safe_child(if enabled { &a } else { &b }, &character, &name)?;
    let other = safe_child(if enabled { &b } else { &a }, &character, &name)?;
    match fs::symlink_metadata(other) { Ok(_) => return Err("两个目录存在同名 Mod".into()), Err(e) if e.kind() == std::io::ErrorKind::NotFound => (), Err(e) => return Err(e.to_string()) }
    let meta = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
    if !meta.is_dir() || is_link(&meta) { return Err("Mod 不是普通目录".into()); }
    let target = dir.join("preview.png");
    match fs::symlink_metadata(&target) { Ok(m) if is_link(&m) || !m.is_file() => return Err("预览图目标不是普通文件".into()), Ok(_) => (), Err(e) if e.kind() == std::io::ErrorKind::NotFound => (), Err(e) => return Err(e.to_string()) }
    let mut temp = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    temp.write_all(&bytes).map_err(|e| format!("写入预览图失败: {e}"))?;
    temp.persist(&target).map_err(|e| format!("保存预览图失败: {e}"))?;
    Ok(())
}

/// 生成拼音全拼与首字母（非汉字字符原样保留小写）。
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

/// 依据文件名中的中文、英文或拼音推断角色（未匹配返回空串）。
pub fn guess_character(candidates: &[String], file_stem: &str) -> String {
    let needle = file_stem.to_lowercase();
    let mut scored: Vec<(usize, usize, &String)> = Vec::new();
    for candidate in candidates {
        let lower = candidate.to_lowercase();
        let (full, initials) = make_pinyin(&lower);
        for (rank, key) in [lower.as_str(), full.as_str(), initials.as_str()].into_iter().enumerate() {
            if key.len() >= 2 && needle.contains(key) {
                scored.push((rank, key.len(), candidate));
                break;
            }
        }
    }
    scored.into_iter().min_by_key(|(rank, len, _)| (*rank, std::cmp::Reverse(*len))).map(|(_, _, name)| name.clone()).unwrap_or_default()
}

/// 在文件管理器中打开当前所在的 Mod 文件夹。
#[tauri::command]
pub fn pak_open_folder(active: String, disabled: String, character: String, name: String, enabled: bool) -> Result<(), String> {
    validate_pak_entry(&active, &disabled, &character, &name, enabled)?;
    let (a, b) = roots(&active, &disabled)?;
    let target = safe_child(if enabled { &a } else { &b }, &character, &name)?;
    tauri_plugin_opener::open_path(&target, None::<&str>).map_err(|e| format!("打开 Mod 文件夹失败: {e}"))
}

/// 将指定状态下的 Mod 文件夹移入系统回收站，不清理关联配置。
#[tauri::command]
pub fn pak_delete(active: String, disabled: String, character: String, name: String, enabled: bool) -> Result<(), String> {
    let _guard = PAK_LOCK.lock().map_err(|e| format!("管理锁不可用: {e}"))?;
    let (a, b) = roots(&active, &disabled)?;
    let (root, other) = if enabled { (&a, &b) } else { (&b, &a) };
    let target = safe_child(root, &character, &name)?;
    let counterpart = safe_child(other, &character, &name)?;
    match fs::symlink_metadata(&counterpart) {
        Ok(_) => return Err("两个目录存在同名 Mod，请先处理冲突；未删除任何文件".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(format!("无法检查另一目录: {e}")),
    }
    let meta = fs::symlink_metadata(&target).map_err(|e| format!("读取 Mod 目录失败: {e}"))?;
    if is_link(&meta) || !meta.is_dir() { return Err("目标不是普通目录或包含重解析点，未删除".into()); }
    trash::delete(&target).map_err(|e| format!("移入回收站失败，未执行永久删除: {e}"))?;
    Ok(())
}

fn verify_copied_tree(source: &Path, target: &Path) -> Result<(), String> {
    use std::io::{BufReader, Read};
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let meta = fs::symlink_metadata(&source_path).map_err(|e| e.to_string())?;
        if is_link(&meta) { return Err("源目录包含链接或重解析点".into()); }
        if meta.is_dir() {
            verify_copied_tree(&source_path, &target_path)?;
        } else if meta.is_file() {
            let mut a = BufReader::new(fs::File::open(&source_path).map_err(|e| e.to_string())?);
            let mut b = BufReader::new(fs::File::open(&target_path).map_err(|e| e.to_string())?);
            let mut left = [0u8; 65536];
            let mut right = [0u8; 65536];
            loop {
                let n = a.read(&mut left).map_err(|e| e.to_string())?;
                let mut read = 0;
                while read < n {
                    let count = b.read(&mut right[read..n]).map_err(|e| e.to_string())?;
                    if count == 0 { return Err("复制校验失败：目标文件不完整".into()); }
                    read += count;
                }
                if left[..n] != right[..n] { return Err("复制校验失败：文件内容不一致".into()); }
                if n == 0 {
                    if b.read(&mut right[..1]).map_err(|e| e.to_string())? != 0 { return Err("复制校验失败：目标文件大小不一致".into()); }
                    break;
                }
            }
        } else { return Err("不支持特殊文件".into()); }
    }
    Ok(())
}

fn move_cross_volume(src: &Path, dst: &Path) -> Result<(), String> {
    let parent = dst.parent().ok_or("无效目标目录")?;
    let staging = tempfile::Builder::new().prefix(".pak-move-").tempdir_in(parent).map_err(|e| format!("创建暂存目录失败: {e}"))?;
    let copied = staging.path().join("content");
    copy_tree(src, &copied)?;
    groups(&copied)?;
    verify_copied_tree(src, &copied)?;
    if dst.exists() { return Err("目标目录已存在，未覆盖；源文件保留".into()); }
    fs::rename(&copied, dst).map_err(|e| format!("完成复制失败，源文件保留: {e}"))?;
    if let Err(e) = verify_copied_tree(src, dst).and_then(|()| verify_copied_tree(dst, src)) {
        return Err(format!("目标已复制，但源目录发生变化或校验失败: {e}。未清理源目录，请检查两侧：{} 和 {}", src.display(), dst.display()));
    }
    if let Err(e) = fs::remove_dir_all(src) {
        return Err(format!("目标已完成复制，但清理源目录失败: {e}。目标副本完整，源目录可能部分保留，请检查：{} 和 {}", src.display(), dst.display()));
    }
    Ok(())
}

/// 返回异环角色重命名映射。
#[tauri::command]
pub fn pak_character_renames() -> std::collections::HashMap<String, String> {
    crate::config::load_config().pak_character_renames
}

/// 列出生效目录中已有角色封面的角色。
#[tauri::command]
pub fn pak_character_covers(active: String, disabled: String) -> Result<Vec<String>, String> {
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (root, _) = roots(&active, &disabled)?;
    let mut names = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if meta.is_dir() && !is_link(&meta) && entry.path().join("_cover.png").is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

/// 将角色封面保存到生效目录的角色根目录。
#[tauri::command]
pub fn pak_set_character_cover(active: String, disabled: String, character: String, image_path: String) -> Result<(), String> {
    use std::io::Write;
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (root, _) = roots(&active, &disabled)?;
    segment(&character)?;
    let dir = root.join(&character);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => (),
        Ok(_) => return Err("角色路径不是普通目录".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&dir).map_err(|e| e.to_string())?,
        Err(e) => return Err(format!("检查角色目录失败: {e}")),
    }
    let bytes = fs::read(image_path).map_err(|e| format!("读取图片失败: {e}"))?;
    if bytes.is_empty() { return Err("图片内容为空".into()); }
    let mut tmp = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    tmp.write_all(&bytes).map_err(|e| e.to_string())?;
    tmp.persist(dir.join("_cover.png")).map_err(|e| format!("保存封面失败: {}", e.error))?;
    Ok(())
}

fn rename_keys<T: Clone>(map: &mut std::collections::HashMap<String, T>, old: &str, new: &str) -> Result<(), String> {
    let prefix = format!("{old}/");
    let updates: Vec<_> = map.iter().filter_map(|(key, value)| key.strip_prefix(&prefix).map(|suffix| (key.clone(), format!("{new}/{suffix}"), value.clone()))).collect();
    if updates.iter().any(|(_, target, _)| map.contains_key(target)) { return Err("新角色名已有相关配置，未覆盖；请先处理冲突".into()); }
    for (old_key, new_key, value) in updates { map.remove(&old_key); map.insert(new_key, value); }
    Ok(())
}

/// 重命名异环角色双目录，并同步预设与配置关联。
#[tauri::command]
pub fn pak_rename_character(active: String, disabled: String, old_name: String, new_name: String) -> Result<(), String> {
    let new_name = new_name.trim();
    segment(&old_name)?; segment(new_name)?;
    if old_name == new_name { return Ok(()); }
    if old_name.to_lowercase() == new_name.to_lowercase() { return Err("暂不支持仅修改大小写的角色重命名".into()); }
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (a, b) = roots(&active, &disabled)?;
    let mut paths = Vec::new();
    for root in [&a, &b] {
        let from = root.join(&old_name); let to = root.join(new_name);
        match fs::symlink_metadata(&to) { Ok(_) => return Err(format!("目标角色目录已存在：{}", to.display())), Err(e) if e.kind() == std::io::ErrorKind::NotFound => (), Err(e) => return Err(e.to_string()) }
        match fs::symlink_metadata(&from) {
            Ok(meta) if meta.is_dir() && !is_link(&meta) => paths.push((from, to)),
            Ok(_) => return Err("角色目录不是普通目录".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    let mut cfg = crate::config::load_config();
    let game = cfg.games.iter_mut().find(|g| g.id == "neverness").ok_or("未配置异环")?;
    rename_keys(&mut game.mod_sections, &old_name, new_name)?;
    rename_keys(&mut game.mod_notes, &old_name, new_name)?;
    rename_keys(&mut game.market_sources, &old_name, new_name)?;
    for s in &mut game.sections { if s.character == old_name { s.character = new_name.to_string(); } }
    for name in &mut game.character_order { if *name == old_name { *name = new_name.to_string(); } }
    let prefix = format!("{old_name}/");
    for key in &mut game.pinned_mods { if let Some(rest) = key.strip_prefix(&prefix) { *key = format!("{new_name}/{rest}"); } }
    for preset in &mut cfg.pak_presets { for entry in &mut preset.entries { if entry.character == old_name { entry.character = new_name.to_string(); } } }
    for value in cfg.pak_character_renames.values_mut() { if *value == old_name { *value = new_name.to_string(); } }
    cfg.pak_character_renames.insert(old_name.clone(), new_name.to_string());
    cfg.pak_character_renames.retain(|key, value| key != value);
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in paths {
        if let Err(e) = fs::rename(&from, &to) {
            let failures: Vec<_> = moved.iter().rev().filter_map(|(src, dst)| fs::rename(dst, src).err().map(|err| format!("{}: {err}", dst.display()))).collect();
            return Err(format!("重命名失败: {e}。回退失败项：{}", if failures.is_empty() { "无".into() } else { failures.join("；") }));
        }
        moved.push((from, to));
    }
    if let Err(e) = crate::config::save_config(&cfg) {
        let failures: Vec<_> = moved.iter().rev().filter_map(|(src, dst)| fs::rename(dst, src).err().map(|err| format!("{}: {err}", dst.display()))).collect();
        return Err(format!("保存关联失败: {e}。目录回退失败项：{}", if failures.is_empty() { "无".into() } else { failures.join("；") }));
    }
    Ok(())
}

/// 列出与普通游戏独立保存的异环预设。
#[tauri::command]
pub fn pak_list_presets() -> Vec<crate::config::PakPreset> {
    crate::config::load_config().pak_presets
}

/// 快照双目录中的 Mod；遇到冲突或扫描错误时拒绝保存。
#[tauri::command]
pub fn pak_save_preset(active: String, disabled: String, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() { return Err("预设名称不能为空".into()); }
    let entries = pak_scan(active, disabled)?;
    for entry in &entries {
        if entry.conflict { return Err(format!("存在同名目录冲突：{}/{}，请处理后再保存", entry.character, entry.name)); }
        if let Some(error) = &entry.error { return Err(format!("无法保存 {}/{}：{}", entry.character, entry.name, error)); }
        segment(&entry.character)?;
        segment(&entry.name)?;
    }
    let mut config = crate::config::load_config();
    config.pak_presets.retain(|preset| preset.name != name);
    config.pak_presets.push(crate::config::PakPreset {
        name: name.to_string(),
        entries: entries.into_iter().map(|entry| crate::config::PakPresetEntry { character: entry.character, name: entry.name, enabled: entry.enabled }).collect(),
        created_at: crate::preset::chrono_now(),
    });
    crate::config::save_config(&config)
}

/// 删除异环预设，不操作 Mod 文件。
#[tauri::command]
pub fn pak_delete_preset(name: String) -> Result<(), String> {
    let mut config = crate::config::load_config();
    let before = config.pak_presets.len();
    config.pak_presets.retain(|preset| preset.name != name);
    if before == config.pak_presets.len() { return Err(format!("预设不存在：{name}")); }
    crate::config::save_config(&config)
}

/// 异环预设应用结果，失败时保留已处理数量和已发现的缺失项。
#[derive(serde::Serialize)]
pub struct PakPresetResult {
    pub moved: usize,
    pub missing: Vec<String>,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct PakPresetCheck {
    pub missing: Vec<String>,
    pub conflicts: Vec<String>,
}

fn check_pak_entries(a: &Path, b: &Path, entries: &[crate::config::PakPresetEntry]) -> Result<PakPresetCheck, String> {
    let mut result = PakPresetCheck { missing: Vec::new(), conflicts: Vec::new() };
    for entry in entries {
        let label = format!("{}/{}", entry.character, entry.name);
        let active = safe_child(a, &entry.character, &entry.name)?;
        let disabled = safe_child(b, &entry.character, &entry.name)?;
        let probe = |path: &Path| match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("检查 {} 失败: {e}", path.display())),
        };
        match (probe(&active)?, probe(&disabled)?) {
            (false, false) => result.missing.push(label),
            (true, true) => result.conflicts.push(label),
            _ => (),
        }
    }
    Ok(result)
}

/// 应用前检查预设中缺失和冲突的 Mod，不修改任何文件。
#[tauri::command]
pub fn pak_check_preset(active: String, disabled: String, name: String) -> Result<PakPresetCheck, String> {
    let config = crate::config::load_config();
    let preset = config.pak_presets.iter().find(|preset| preset.name == name).ok_or("预设不存在")?;
    let _guard = PAK_LOCK.lock().map_err(|e| format!("管理锁不可用: {e}"))?;
    let (a, b) = roots(&active, &disabled)?;
    check_pak_entries(&a, &b, &preset.entries)
}

fn apply_pak_entries(a: &Path, b: &Path, entries: &[crate::config::PakPresetEntry], skip_conflicts: &[String]) -> PakPresetResult {
    let mut result = PakPresetResult { moved: 0, missing: Vec::new(), error: None };
    for entry in entries {
        let label = format!("{}/{}", entry.character, entry.name);
        if skip_conflicts.contains(&label) { continue; }
        let step = (|| -> Result<bool, String> {
            let active = safe_child(a, &entry.character, &entry.name)?;
            let disabled = safe_child(b, &entry.character, &entry.name)?;
            let probe = |path: &Path| -> Result<bool, String> {
                match fs::symlink_metadata(path) {
                    Ok(_) => Ok(true),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                    Err(e) => Err(format!("检查目录失败: {e}")),
                }
            };
            match (probe(&active)?, probe(&disabled)?) {
                (false, false) => { result.missing.push(label.clone()); Ok(false) },
                (true, true) => Err("出现未确认的同名目录冲突，未覆盖，请重新检查预设".into()),
                (true, false) if entry.enabled => Ok(false),
                (false, true) if !entry.enabled => Ok(false),
                _ => { toggle_locked(a, b, &entry.character, &entry.name, entry.enabled)?; Ok(true) },
            }
        })();
        match step {
            Ok(true) => result.moved += 1,
            Ok(false) => (),
            Err(error) => { result.error = Some(format!("{label}：{error}")); break; }
        }
    }
    result
}

/// 应用预设中记录的 Mod；未记录的 Mod 保持原状。
#[tauri::command]
pub fn pak_apply_preset(active: String, disabled: String, name: String, skip_conflicts: Vec<String>) -> Result<PakPresetResult, String> {
    let config = crate::config::load_config();
    let preset = config.pak_presets.iter().find(|preset| preset.name == name).ok_or("预设不存在")?;
    let _guard = PAK_LOCK.lock().map_err(|e| format!("管理锁不可用: {e}"))?;
    let (a, b) = roots(&active, &disabled)?;
    Ok(apply_pak_entries(&a, &b, &preset.entries, &skip_conflicts))
}

#[tauri::command]
pub fn pak_toggle(active: String, disabled: String, character: String, name: String, enable: bool) -> Result<(), String> {
    let _guard = PAK_LOCK.lock().map_err(|e| e.to_string())?;
    let (a,b) = roots(&active, &disabled)?;
    toggle_locked(&a, &b, &character, &name, enable)
}

fn toggle_locked(a: &Path, b: &Path, character: &str, name: &str, enable: bool) -> Result<(), String> {
    let (src_root,dst_root) = if enable { (b,a) } else { (a,b) };
    let src = safe_child(src_root, &character, &name)?; let dst = safe_child(dst_root, &character, &name)?;
    if is_link(&fs::symlink_metadata(&src).map_err(|e| format!("读取源目录失败: {e}"))?) { return Err("不支持链接或重解析点目录".into()); }
    groups(&src)?; if dst.exists() { return Err("目标目录已存在，未覆盖".into()); }
    fs::create_dir_all(dst.parent().ok_or("无效目标")?).map_err(|e| e.to_string())?;
    match fs::rename(&src, &dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => move_cross_volume(&src, &dst),
        Err(e) => Err(format!("移动失败，原文件保留: {e}")),
    }
}

//! Persist Bridge —— 精确复刻原 QAQManager 的 mod 部件状态持久化机制。
//!
//! 核心原理：把每个 mod 里 `global persist $xxx` 声明的「部件切换变量」
//! 抽到托管命名空间 `QAQM\Persist\Bridge_<hash>`，由管理器持久化保存，
//! 使 mod 被禁用/切走再切回后，部件状态（变量值）不丢失。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

// ===== 常量（与原作者完全一致） =====
const PERSIST_BRIDGE_STATE_FILE_PREFIX: &str = "qaqm_state_";
const PERSIST_ACTIVE_INCLUDE_FILE: &str = "qaqm_persist_active.ini";
const QAQM_BRIDGE_DIR: &str = "qaqm";
const QAQM_INCLUDER_FILE: &str = "qaqm_includer.ini";
const QAQM_KEYPRESS_FILE: &str = "qaqm_keypress.ini";
const PERSIST_BRIDGE_STATE_DIR: &str = "cache";

// ===== 路径工具 =====

/// 规范化相对路径：/ → \，去空段，逐段去 DISABLED_ 前缀
fn normalize_relative_path(value: &str) -> String {
    value
        .replace('/', "\\")
        .split('\\')
        .filter(|s| !s.is_empty())
        .map(|seg| {
            if let Some(stripped) = seg
                .strip_prefix("DISABLED_")
                .or_else(|| seg.strip_prefix("disabled_"))
            {
                stripped.to_string()
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\\")
}

/// 计算 mod 的 persist descriptor（hash、state 文件名、命名空间）
#[allow(dead_code)]
struct PersistDescriptor {
    hash: String,
    canonical_relative_path: String,
    state_file_name: String,
    namespace: String,
}

fn build_descriptor(mods_root: &Path, ini_path: &Path) -> PersistDescriptor {
    let rel = ini_path.strip_prefix(mods_root).unwrap_or(ini_path);
    let canonical = normalize_relative_path(&rel.to_string_lossy());
    let hash = {
        let mut hasher = Sha1::new();
        hasher.update(canonical.to_lowercase().as_bytes());
        let digest = hasher.finalize();
        let hex = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
        hex[..12].to_string()
    };
    PersistDescriptor {
        state_file_name: format!("{PERSIST_BRIDGE_STATE_FILE_PREFIX}{hash}.ini"),
        namespace: format!("QAQM\\Persist\\Bridge_{hash}"),
        hash,
        canonical_relative_path: canonical,
    }
}

// ===== ini 解析 =====

/// 解析 ini 里的 global persist 变量声明
/// 返回 (变量名, 默认值)，按小写变量名去重
fn parse_local_persist_declarations(content: &str) -> Vec<(String, String)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut decls: Vec<(String, String)> = Vec::new();
    for line in content.replace("\r\n", "\n").split('\n') {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("global persist $") {
            continue;
        }
        // 匹配 global persist $var = value
        let rest = trimmed.trim_start_matches("global persist").trim_start();
        if let Some(rest) = rest.strip_prefix('$') {
            let eq_pos = rest.find('=');
            if let Some(eq) = eq_pos {
                let var = rest[..eq].trim();
                let val = rest[eq + 1..].trim();
                // 校验变量名
                if is_valid_var_name(var) {
                    let norm = var.to_lowercase();
                    if seen.insert(norm) {
                        decls.push((var.to_string(), val.to_string()));
                    }
                }
            }
        }
    }
    decls
}

fn is_valid_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 解析 namespace = xxx
fn parse_ini_namespace(content: &str) -> Option<String> {
    for line in content.replace("\r\n", "\n").split('\n') {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("namespace") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let val = rest.trim();
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// 提取托管注释 `; Persisted state is hosted by <file>` 里的文件名
fn extract_hosted_state_files(content: &str) -> Vec<String> {
    let mut files = HashSet::new();
    for line in content.replace("\r\n", "\n").split('\n') {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(';') {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix("Persisted state is hosted by") {
                let fname = rest.trim();
                if is_valid_state_file_name(fname) {
                    files.insert(fname.to_string());
                }
            }
        }
    }
    let mut v: Vec<String> = files.into_iter().collect();
    v.sort();
    v
}

fn is_valid_state_file_name(s: &str) -> bool {
    s.starts_with(PERSIST_BRIDGE_STATE_FILE_PREFIX) && s.ends_with(".ini")
}

/// 判断内容是否含托管引用
fn has_qaqm_refs(content: &str) -> bool {
    content.contains("\\QAQM\\Persist\\")
        || content.to_lowercase().contains("persisted state is hosted by qaqm_state_")
}

// ===== mod.ini 改写 =====

/// 把 mod.ini 里的 $xxx 引用改写成 $\<namespace>\<xxx>，并删除全局声明行
fn apply_persist_bridge(content: &str, descriptor: &PersistDescriptor, decls: &[(String, String)]) -> String {
    let persist_names: HashSet<String> = decls.iter().map(|(v, _)| v.to_lowercase()).collect();
    let mut next_lines: Vec<String> = Vec::new();
    let mut inserted_comment = false;

    for line in content.replace("\r\n", "\n").split('\n') {
        // 是否 global persist 声明行
        let is_decl_line = {
            let trimmed = line.trim_start();
            if trimmed.starts_with("global persist $") {
                let rest = trimmed.trim_start_matches("global persist").trim_start();
                if let Some(rest) = rest.strip_prefix('$') {
                    if let Some(eq) = rest.find('=') {
                        let var = rest[..eq].trim().to_lowercase();
                        persist_names.contains(&var)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };

        if is_decl_line {
            if !inserted_comment {
                next_lines.push(format!("; Persisted state is hosted by {}", descriptor.state_file_name));
                inserted_comment = true;
            }
            continue; // 删除声明行
        }

        // 节标题行原样保留
        if line.trim_start().starts_with('[') {
            next_lines.push(line.to_string());
            continue;
        }

        // 替换 $var 引用 → $\namespace\var
        let mut new_line = line.to_string();
        for (var, _) in decls {
            new_line = replace_var_refs(&new_line, var, &descriptor.namespace);
        }
        next_lines.push(new_line);
    }
    next_lines.join("\n")
}

/// 在单行内替换 $var 引用（负向后顾：$ 前面不能是字母/数字/下划线/反斜杠）
fn replace_var_refs(line: &str, var: &str, namespace: &str) -> String {
    let replacement = format!("$\\{namespace}\\{var}");
    let mut out = String::new();
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '$' {
            // 检查前一个字符
            let prev_ok = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == '_' || bytes[i - 1] == '\\');
            // 尝试匹配 var
            let rest: String = bytes[i + 1..].iter().collect();
            if prev_ok && rest.starts_with(var) {
                // 检查 var 后面的字符是单词边界
                let after = bytes.get(i + 1 + var.len());
                let boundary_ok = match after {
                    None => true,
                    Some(c) => !(c.is_ascii_alphanumeric() || *c == '_'),
                };
                if boundary_ok {
                    out.push_str(&replacement);
                    i += 1 + var.len();
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

// ===== 状态文件读写 =====

/// 构建 qaqm_state_<hash>.ini 的内容
fn build_state_content(
    descriptor: &PersistDescriptor,
    decls: &[(String, String)],
    existing_values: &HashMap<String, String>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("namespace = {}", descriptor.namespace));
    lines.push(String::new());
    lines.push("[Constants]".to_string());
    for (var, default) in decls {
        let value = existing_values.get(&var.to_lowercase()).cloned().unwrap_or_else(|| default.clone());
        lines.push(format!("global persist ${var} = {value}"));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// 读取现有 state 文件的变量值（key 为小写变量名）
fn read_state_values(state_path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(content) = fs::read_to_string(state_path) {
        for (var, val) in parse_local_persist_declarations(&content) {
            map.insert(var.to_lowercase(), val);
        }
    }
    map
}

/// 获取 bridge 目录路径
fn bridge_dir(root: &Path) -> PathBuf {
    root.join(QAQM_BRIDGE_DIR)
}

/// 获取 state 缓存目录路径
fn state_dir(root: &Path) -> PathBuf {
    bridge_dir(root).join(PERSIST_BRIDGE_STATE_DIR)
}

/// 获取 state 文件完整路径
fn state_path(root: &Path, file_name: &str) -> PathBuf {
    state_dir(root).join(file_name)
}

/// 确保 bridge 目录存在
fn ensure_bridge_dir(root: &Path) -> Result<PathBuf, String> {
    let dir = bridge_dir(root);
    fs::create_dir_all(&dir).map_err(|e| format!("创建 qaqm 目录失败: {e}"))?;
    Ok(dir)
}

/// 确保 state 缓存目录存在
fn ensure_state_dir(root: &Path) -> Result<PathBuf, String> {
    let dir = state_dir(root);
    fs::create_dir_all(&dir).map_err(|e| format!("创建 cache 目录失败: {e}"))?;
    Ok(dir)
}

// ===== d3dx_user.ini constants 读写 =====

/// 读取 d3dx_user.ini 里的 [Constants] 键值（key 规范化为小写、去开头反斜杠）
pub fn read_d3dx_user_constants(d3dx_user_path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let content = match fs::read_to_string(d3dx_user_path) {
        Ok(c) => c,
        Err(_) => return map,
    };
    let mut in_constants = false;
    for line in content.replace("\r\n", "\n").split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_constants = trimmed.starts_with("[Constants]");
            continue;
        }
        if !in_constants {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim();
            let value = trimmed[eq + 1..].trim();
            let norm_key = normalize_d3dx_user_key(key);
            map.insert(norm_key, value.to_string());
        }
    }
    map
}

/// 规范化 d3dx_user key：/→\，去开头反斜杠，小写
fn normalize_d3dx_user_key(value: &str) -> String {
    value
        .replace('/', "\\")
        .trim_start_matches('\\')
        .to_lowercase()
}

// ===== 枚举 ini 文件 =====

/// 递归收集目录下所有 .ini 文件
pub fn get_ini_files(dir: &Path) -> Vec<PathBuf> {
    let mut list = Vec::new();
    collect_ini_files(dir, &mut list);
    list
}

fn collect_ini_files(dir: &Path, list: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_ini_files(&p, list);
        } else if p.extension().map(|e| e.to_string_lossy().to_lowercase()) == Some(".ini".to_string()) {
            list.push(p);
        }
    }
}

// ===== 判断 mod 是否被托管 =====

/// 判断一个 mod 目录是否已经用了 persist bridge
pub fn dir_uses_managed_persist_bridge(mod_dir: &Path) -> bool {
    if !mod_dir.is_dir() {
        return false;
    }
    for ini in get_ini_files(mod_dir) {
        if let Ok(content) = fs::read_to_string(&ini) {
            if has_qaqm_refs(&content) {
                return true;
            }
        }
    }
    false
}

/// 收集一个 mod 目录下所有被托管的 state 文件名
pub fn collect_hosted_state_files(mod_dir: &Path) -> Vec<String> {
    let mut files = HashSet::new();
    for ini in get_ini_files(mod_dir) {
        if let Ok(content) = fs::read_to_string(&ini) {
            for f in extract_hosted_state_files(&content) {
                files.insert(f);
            }
        }
    }
    let mut v: Vec<String> = files.into_iter().collect();
    v.sort();
    v
}

// ===== 核心迁移：托管一个 mod 目录 =====

/// 对一个 mod 目录执行 persist bridge 托管（等价于原 ensurePersistBridgeMigrationForModDir）
/// root = 游戏根目录（Mods 的上一层）
pub fn migrate_mod_dir(root: &Path, mod_dir: &Path) -> Result<usize, String> {
    let mods_root = root.join("Mods");
    let d3dx_user_path = root.join("d3dx_user.ini");
    let d3dx_constants = read_d3dx_user_constants(&d3dx_user_path);

    let ini_files = get_ini_files(mod_dir);
    let mut generated_count = 0usize;
    let mut bridge_mappings: Vec<(String, String, Vec<String>)> = Vec::new();

    for ini_path in &ini_files {
        // 处理已存在的备份：若当前含 qaqm 引用则从备份还原声明，否则删备份
        let backup_path = PathBuf::from(format!("{}.qaqm-persistbak", ini_path.display()));
        if backup_path.exists() {
            if let Ok(current_raw) = fs::read_to_string(ini_path) {
                if has_qaqm_refs(&current_raw) {
                    // 从备份还原
                    if let Ok(bak) = fs::read_to_string(&backup_path) {
                        let _ = fs::write(ini_path, &bak);
                    }
                } else {
                    let _ = fs::remove_file(&backup_path);
                }
            }
        }

        let content = match fs::read_to_string(ini_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let decls = parse_local_persist_declarations(&content);
        if decls.is_empty() {
            continue;
        }

        let descriptor = build_descriptor(&mods_root, ini_path);
        let sp = state_path(root, &descriptor.state_file_name);
        let mut existing_values = read_state_values(&sp);

        // 从 d3dx_user.ini 补充当前值
        for (var, _) in &decls {
            let raw_key = format!("$\\{}\\{var}", descriptor.namespace);
            let norm = normalize_d3dx_user_key(&raw_key);
            if let Some(v) = d3dx_constants.get(&norm) {
                existing_values.insert(var.to_lowercase(), v.clone());
            }
        }

        let state_content = build_state_content(&descriptor, &decls, &existing_values);
        ensure_state_dir(root)?;
        if write_if_changed(&sp, &state_content)? {
            generated_count += 1;
        }

        if let Some(orig_ns) = parse_ini_namespace(&content) {
            bridge_mappings.push((
                orig_ns,
                descriptor.namespace.clone(),
                decls.iter().map(|(v, _)| v.clone()).collect(),
            ));
        }

        let next = apply_persist_bridge(&content, &descriptor, &decls);
        if next != content {
            if !backup_path.exists() {
                let _ = fs::write(&backup_path, &content);
            }
            let _ = fs::write(ini_path, &next);
        }
    }

    // 跨文件引用改写
    if !bridge_mappings.is_empty() {
        for ini_path in &ini_files {
            let content = match fs::read_to_string(ini_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let updated = apply_external_bridge_refs(&content, &bridge_mappings);
            if updated != content {
                let backup_path = PathBuf::from(format!("{}.qaqm-persistbak", ini_path.display()));
                if !backup_path.exists() {
                    let _ = fs::write(&backup_path, &content);
                }
                let _ = fs::write(ini_path, &updated);
            }
        }
    }

    if generated_count > 0 {
        rebuild_active_includes(root)?;
    }

    Ok(generated_count)
}

/// 跨文件引用改写：$\<orig_ns>\<var> → $\<bridge_ns>\<var>
fn apply_external_bridge_refs(
    content: &str,
    mappings: &[(String, String, Vec<String>)],
) -> String {
    let mut result = content.to_string();
    for (orig_ns, bridge_ns, var_names) in mappings {
        if orig_ns.is_empty() || var_names.is_empty() {
            continue;
        }
        for var in var_names {
            let from = format!("$\\{orig_ns}\\{var}");
            let to = format!("$\\{bridge_ns}\\{var}");
            result = result.replace(&from, &to);
        }
    }
    result
}

/// 内容变化才写盘
fn write_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing == content {
        return Ok(false);
    }
    fs::write(path, content).map_err(|e| format!("写入失败 {}: {e}", path.display()))?;
    Ok(true)
}

// ===== include 段生成 =====

/// 缓存保留数量上限（与原 QAQM 一致：0-50）
pub const MAX_CACHE_SIZE: usize = 50;

/// 从配置读取缓存保留数量（clamp 到 0-50）
fn cache_size_limit() -> usize {
    let cfg = crate::config::load_config();
    cfg.persist_cache_size.min(MAX_CACHE_SIZE)
}

/// 从 state 文件名生成 include 段名
fn state_include_section_name(file_name: &str) -> String {
    let base = file_name.trim_end_matches(".ini");
    let sanitized: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("IncludePersistBridge_{sanitized}")
}

/// 确保 include 条目存在
fn ensure_include_entry(content: &str, section: &str, target: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    // 已存在则返回
    let include_line = format!("include = {target}");
    if normalized.lines().any(|l| l.trim() == include_line) {
        return if normalized.ends_with('\n') { normalized } else { format!("{normalized}\n") };
    }
    let trimmed = normalized.trim_end();
    let block = format!("[{section}]\n{include_line}\n");
    if trimmed.is_empty() {
        block
    } else {
        format!("{trimmed}\n\n{block}")
    }
}

/// 生成 qaqm_persist_active.ini
fn write_active_include(
    root: &Path,
    enabled_files: &[String],
    recent_files: &[String],
    cache_size: usize,
) -> Result<(), String> {
    ensure_bridge_dir(root)?;
    let trimmed_recent: Vec<String> = if cache_size > 0 {
        recent_files.iter().rev().take(cache_size).rev().cloned().collect()
    } else {
        Vec::new()
    };
    let mut final_files: Vec<String> = enabled_files.to_vec();
    for f in &trimmed_recent {
        if !final_files.contains(f) {
            final_files.push(f.clone());
        }
    }

    let mut content = String::new();
    for f in &final_files {
        let target = format!("{PERSIST_BRIDGE_STATE_DIR}\\{f}");
        content = ensure_include_entry(&content, &state_include_section_name(f), &target);
    }
    let path = bridge_dir(root).join(PERSIST_ACTIVE_INCLUDE_FILE);
    write_if_changed(&path, &content)?;
    Ok(())
}

/// 重建 qaqm_persist_active.ini：扫描所有启用的 mod，收集其托管的 state 文件
pub fn rebuild_active_includes(root: &Path) -> Result<(), String> {
    let mods_root = root.join("Mods");
    let mut enabled: HashSet<String> = HashSet::new();
    if mods_root.is_dir() {
        if let Ok(chars) = fs::read_dir(&mods_root) {
            for char_entry in chars.flatten() {
                let char_path = char_entry.path();
                if !char_path.is_dir() {
                    continue;
                }
                if let Ok(mods) = fs::read_dir(&char_path) {
                    for mod_entry in mods.flatten() {
                        let name = mod_entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("DISABLED_") || !mod_entry.path().is_dir() {
                            continue;
                        }
                        for f in collect_hosted_state_files(&mod_entry.path()) {
                            enabled.insert(f);
                        }
                    }
                }
            }
        }
    }

    // recent = 现有 include 里非启用的
    let active_path = bridge_dir(root).join(PERSIST_ACTIVE_INCLUDE_FILE);
    let mut recent: Vec<String> = Vec::new();
    if let Ok(content) = fs::read_to_string(&active_path) {
        for line in content.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("include = ") {
                let fname = rest.trim();
                if let Some(f) = fname.split('\\').last() {
                    if is_valid_state_file_name(f) && !enabled.contains(f) {
                        recent.push(f.to_string());
                    }
                }
            }
        }
    }

    let mut enabled_vec: Vec<String> = enabled.into_iter().collect();
    enabled_vec.sort();
    write_active_include(root, &enabled_vec, &recent, cache_size_limit())?;
    Ok(())
}

/// 确保 qaqm_includer.ini 挂到 d3dx.ini
pub fn ensure_includer_mounted(root: &Path) -> Result<(), String> {
    ensure_bridge_dir(root)?;
    let includer_path = bridge_dir(root).join(QAQM_INCLUDER_FILE);

    // 生成 includer 内容
    let mut content = String::new();
    content = ensure_include_entry(&content, "IncludeKeypress", QAQM_KEYPRESS_FILE);
    content = ensure_include_entry(&content, "IncludePersistBridgeActive", PERSIST_ACTIVE_INCLUDE_FILE);
    fs::write(&includer_path, &content).map_err(|e| format!("写 includer 失败: {e}"))?;

    // 挂到 d3dx.ini
    let d3dx_path = root.join("d3dx.ini");
    if !d3dx_path.exists() {
        return Err("d3dx.ini 不存在".into());
    }
    let d3dx_content = fs::read_to_string(&d3dx_path).map_err(|e| format!("读 d3dx.ini 失败: {e}"))?;
    let include_line = "include = qaqm\\qaqm_includer.ini";
    if !d3dx_content.lines().any(|l| l.trim() == include_line) {
        // 备份
        let bak = root.join("d3dx.ini.qaqmbak");
        if !bak.exists() {
            let _ = fs::copy(&d3dx_path, &bak);
        }
        let new_content = ensure_include_entry(&d3dx_content, "Include", include_line.trim());
        fs::write(&d3dx_path, new_content).map_err(|e| format!("写 d3dx.ini 失败: {e}"))?;
    }
    Ok(())
}

// ===== 公共 API（供 toggle 等调用） =====

/// 迁移所有启用的 mod（遍历 Mods/角色/非DISABLED_mod，逐个托管）
pub fn migrate_all_mods(root: &Path) -> Result<usize, String> {
    let mods_root = root.join("Mods");
    if !mods_root.is_dir() {
        return Ok(0);
    }
    let mut total = 0usize;
    if let Ok(chars) = fs::read_dir(&mods_root) {
        for char_entry in chars.flatten() {
            let char_path = char_entry.path();
            if !char_path.is_dir() {
                continue;
            }
            if let Ok(mods) = fs::read_dir(&char_path) {
                for mod_entry in mods.flatten() {
                    let name = mod_entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("DISABLED_") || !mod_entry.path().is_dir() {
                        continue;
                    }
                    if let Ok(n) = migrate_mod_dir(root, &mod_entry.path()) {
                        total += n;
                    }
                }
            }
        }
    }
    Ok(total)
}

/// 还原所有 mod（把 .qaqm-persistbak 还原回原文件）
pub fn restore_all_mods(root: &Path) -> Result<usize, String> {
    let mods_root = root.join("Mods");
    if !mods_root.is_dir() {
        return Ok(0);
    }
    let mut total = 0usize;
    if let Ok(chars) = fs::read_dir(&mods_root) {
        for char_entry in chars.flatten() {
            let char_path = char_entry.path();
            if !char_path.is_dir() {
                continue;
            }
            if let Ok(mods) = fs::read_dir(&char_path) {
                for mod_entry in mods.flatten() {
                    if mod_entry.path().is_dir() {
                        total += restore_persist_bak_files(&mod_entry.path());
                    }
                }
            }
        }
    }
    Ok(total)
}

/// 递归还原目录下所有 .qaqm-persistbak
fn restore_persist_bak_files(dir: &Path) -> usize {
    let mut count = 0;
    walk_restore(dir, &mut count);
    count
}

/// 还原单个 mod 目录下的 .qaqm-persistbak（导入整合包后调用）
pub(crate) fn restore_bak_in_dir(dir: &Path) -> usize {
    restore_persist_bak_files(dir)
}

fn walk_restore(dir: &Path, count: &mut usize) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_restore(&p, count);
        } else if p
            .file_name()
            .map(|n| n.to_string_lossy().ends_with(".qaqm-persistbak"))
            .unwrap_or(false)
        {
            // 还原：去掉 .qaqm-persistbak 后缀
            let original_str = p.to_string_lossy();
            let original_path = PathBuf::from(original_str.trim_end_matches(".qaqm-persistbak"));
            if original_path.exists() {
                let _ = fs::remove_file(&original_path);
            }
            if fs::rename(&p, &original_path).is_ok() {
                *count += 1;
            }
        }
    }
}

/// 读取某个 mod 目录的 persist 变量值（变量名 → 当前值）
pub fn read_mod_persist_values(mods_root: &Path, mod_dir: &Path) -> Result<Vec<(String, String)>, String> {
    if !mod_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut result: Vec<(String, String)> = Vec::new();
    // 读取该 mod 的托管 state 文件
    for state_file in collect_hosted_state_files(mod_dir) {
        // state 文件在 mods_root 的父目录的 qaqm/cache 下
        let root = mods_root.parent().unwrap_or(mods_root);
        let sp = state_path(root, &state_file);
        if let Ok(content) = fs::read_to_string(&sp) {
            for (var, val) in parse_local_persist_declarations(&content) {
                result.push((var, val));
            }
        }
    }
    // 如果没读到（未托管），尝试从 d3dx_user.ini 读
    if result.is_empty() {
        let root = mods_root.parent().unwrap_or(mods_root);
        let d3dx_user = root.join("d3dx_user.ini");
        let constants = read_d3dx_user_constants(&d3dx_user);
        // 计算该 mod 各 ini 的 descriptor，读对应变量
        for ini in get_ini_files(mod_dir) {
            let descriptor = build_descriptor(mods_root, &ini);
            let prefix = descriptor.namespace.to_lowercase();
            for (k, v) in &constants {
                if k.starts_with(&prefix) {
                    // key 格式 $\namespace\var → 提取 var
                    if let Some(var) = k.rsplit('\\').next() {
                        result.push((var.to_string(), v.clone()));
                    }
                }
            }
        }
    }
    result.sort();
    result.dedup();
    Ok(result)
}

/// 清空 persist 缓存：删除 cache 目录下所有 qaqm_state_*.ini，清空 persist_active.ini
pub fn clear_cache(root: &Path) -> Result<usize, String> {
    let dir = state_dir(root);
    let mut count = 0;
    if dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_valid_state_file_name(&name) {
                    let _ = fs::remove_file(entry.path());
                    count += 1;
                }
            }
        }
    }
    // 清空 persist_active.ini
    let active_path = bridge_dir(root).join(PERSIST_ACTIVE_INCLUDE_FILE);
    let _ = fs::write(&active_path, "");
    Ok(count)
}

/// 判断一个 mod 目录是否被托管（对外）
#[allow(dead_code)]
pub fn mod_uses_persist(mod_dir: &Path) -> bool {
    dir_uses_managed_persist_bridge(mod_dir)
}

/// 启用/禁用 mod 时统一联动（root=游戏根目录，mod_dir=mod 目录）
/// 注意：禁用前会先同步 d3dx_user 的最新值到 state 文件
pub fn sync_mod_toggle(root: &Path, mod_dir: &Path, enabled: bool) -> Result<(), String> {
    if enabled {
        on_mod_enabled(root, mod_dir)
    } else {
        // 禁用前先同步状态（把 d3dx_user 的最新值写回 state 文件）
        let _ = sync_persist_state_for_mod(root, mod_dir);
        on_mod_disabled(root, mod_dir)
    }
}

/// 把该 mod 托管 state 文件里每个变量的值，回写成 d3dx_user.ini 中的当前值。
/// 等价原 syncPersistBridgeStateForModDir：禁用前把最新值落盘。
fn sync_persist_state_for_mod(root: &Path, mod_dir: &Path) -> Result<usize, String> {
    if !mod_dir.is_dir() {
        return Ok(0);
    }
    let d3dx_user_path = root.join("d3dx_user.ini");
    if !d3dx_user_path.exists() {
        return Ok(0);
    }
    let state_files = collect_hosted_state_files(mod_dir);
    if state_files.is_empty() {
        return Ok(0);
    }
    let constants = read_d3dx_user_constants(&d3dx_user_path);
    let mut updated = 0usize;

    for state_file in state_files {
        let sp = state_path(root, &state_file);
        if !sp.exists() {
            continue;
        }
        let content = match fs::read_to_string(&sp) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // 读 namespace
        let namespace = parse_ini_namespace(&content);
        let Some(namespace) = namespace else { continue };

        let mut changed = false;
        let mut next_lines: Vec<String> = Vec::new();
        for line in content.replace("\r\n", "\n").split('\n') {
            let trimmed = line.trim();
            // 匹配 global persist $var = value
            let mut new_line = line.to_string();
            if let Some(rest) = trimmed.strip_prefix("global persist $") {
                if let Some(eq) = rest.find('=') {
                    let var = rest[..eq].trim();
                    if is_valid_var_name(var) {
                        let raw_key = format!("$\\{namespace}\\{var}");
                        let norm = normalize_d3dx_user_key(&raw_key);
                        if let Some(v) = constants.get(&norm) {
                            let next = format!("global persist ${var} = {v}");
                            if next != trimmed {
                                new_line = next;
                                changed = true;
                            }
                        }
                    }
                }
            }
            next_lines.push(new_line);
        }
        if changed {
            let out = next_lines.join("\n").trim_end().to_string() + "\n";
            fs::write(&sp, &out).map_err(|e| format!("写 state 文件失败: {e}"))?;
            updated += 1;
        }
    }
    Ok(updated)
}

/// 启用 mod 后：把它的 state 文件加入 enabled（增量）
pub fn on_mod_enabled(root: &Path, mod_dir: &Path) -> Result<(), String> {
    let state_files = collect_hosted_state_files(mod_dir);
    if state_files.is_empty() {
        return Ok(());
    }
    // 读取现有 include，合并
    let active_path = bridge_dir(root).join(PERSIST_ACTIVE_INCLUDE_FILE);
    let mut enabled: Vec<String> = Vec::new();
    let mut recent: Vec<String> = Vec::new();
    if let Ok(content) = fs::read_to_string(&active_path) {
        for line in content.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("include = ") {
                if let Some(f) = rest.trim().split('\\').last() {
                    if is_valid_state_file_name(f) {
                        enabled.push(f.to_string());
                    }
                }
            }
        }
    }
    // 把新启用的加入
    for f in state_files {
        // 从 recent 移除
        recent.retain(|r| r != &f);
        if !enabled.contains(&f) {
            enabled.push(f);
        }
    }
    enabled.sort();
    write_active_include(root, &enabled, &recent, cache_size_limit())
}

/// 禁用 mod 后：把它的 state 文件移到 recent（缓存）
pub fn on_mod_disabled(root: &Path, mod_dir: &Path) -> Result<(), String> {
    let state_files = collect_hosted_state_files(mod_dir);
    if state_files.is_empty() {
        return Ok(());
    }
    let active_path = bridge_dir(root).join(PERSIST_ACTIVE_INCLUDE_FILE);
    let mut enabled: Vec<String> = Vec::new();
    let mut recent: Vec<String> = Vec::new();
    if let Ok(content) = fs::read_to_string(&active_path) {
        for line in content.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("include = ") {
                if let Some(f) = rest.trim().split('\\').last() {
                    if is_valid_state_file_name(f) {
                        enabled.push(f.to_string());
                    }
                }
            }
        }
    }
    // 从 enabled 移除，加入 recent
    for f in &state_files {
        enabled.retain(|e| e != f);
        if !recent.contains(f) {
            recent.push(f.clone());
        }
    }
    enabled.sort();
    write_active_include(root, &enabled, &recent, cache_size_limit())
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_relative_path() {
        assert_eq!(normalize_relative_path("a/b\\c//d"), "a\\b\\c\\d");
        assert_eq!(normalize_relative_path("DISABLED_foo\\bar"), "foo\\bar");
        assert_eq!(normalize_relative_path("x\\DISABLED_y\\z"), "x\\y\\z");
    }

    #[test]
    fn test_hash_matches_known() {
        // 验证 hash 算法稳定性：长度与格式
        let d = build_descriptor(
            Path::new("D:\\XXMI MODS\\EndField Game\\Mods"),
            Path::new("D:\\XXMI MODS\\EndField Game\\Mods\\角色A\\示例Mod\\Ember.ini"),
        );
        assert_eq!(d.hash.len(), 12);
        assert_eq!(d.namespace, format!("QAQM\\Persist\\Bridge_{}", d.hash));
        assert_eq!(d.state_file_name, format!("qaqm_state_{}.ini", d.hash));
    }

    #[test]
    fn test_parse_persist_declarations() {
        let content = "global persist $skirt = 0\nglobal persist $SKIRT = 1\nglobal persist $hair = 0";
        let decls = parse_local_persist_declarations(content);
        // $SKIRT 和 $skirt 小写相同，去重保留第一个
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].0, "skirt");
        assert_eq!(decls[0].1, "0");
    }

    #[test]
    fn test_apply_persist_bridge() {
        let content = "namespace = Foo\n[Constants]\nglobal persist $skirt = 0\n$skirt = 1\n[Draw]\ndraw = $skirt";
        let decls = parse_local_persist_declarations(content);
        let d = PersistDescriptor {
            hash: "abc123".to_string(),
            state_file_name: "qaqm_state_abc123.ini".to_string(),
            namespace: "QAQM\\Persist\\Bridge_abc123".to_string(),
            canonical_relative_path: "test".to_string(),
        };
        let out = apply_persist_bridge(content, &d, &decls);
        assert!(out.contains("; Persisted state is hosted by qaqm_state_abc123.ini"));
        assert!(!out.contains("global persist $skirt"));
        assert!(out.contains("$\\QAQM\\Persist\\Bridge_abc123\\skirt"));
    }
}

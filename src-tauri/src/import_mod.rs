//! Mod 导入：解压压缩包（zip/rar/7z）并放入对应角色目录。
//!
//! 解压全部由内置依赖完成，运行时不依赖系统的 7-Zip / Bandizip / WinRAR：
//! - zip → `zip` crate
//! - 7z  → `sevenz-rust2` crate
//! - rar → `unrar` crate（静态链接 UnRAR C++ 源码）
//!
//! 依据 UnRAR 许可要求，在此声明：
//! UnRAR source code may be used in any software to handle RAR archives
//! without limitations free of charge, but cannot be used to develop RAR
//! (WinRAR) compatible archiver and to re-create RAR compression algorithm,
//! which is proprietary. Copyright of RAR and UnRAR belongs to Alexander Roshal.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 导入候选（拖拽/批量导入时用于预览）
#[derive(Debug, Clone, Serialize)]
pub struct ImportCandidate {
    /// 压缩包绝对路径
    pub path: String,
    /// 建议的 mod 名称（压缩包文件名去扩展）
    pub name: String,
    /// 自动匹配到的角色（空字符串表示未匹配）
    pub character: String,
    /// 文件大小（字节）
    pub size: u64,
}

/// 分析待导入的压缩包：提取名称、自动匹配角色
pub fn analyze_imports(mods_path: &str, paths: &[String]) -> Vec<ImportCandidate> {
    // 已有角色目录
    let mut chars: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(mods_path) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let n = e.file_name().to_string_lossy().to_string();
                if !n.starts_with("DISABLED_") && !n.to_lowercase().starts_with("disabled") {
                    chars.push(n);
                }
            }
        }
    }
    // 长名优先匹配，避免 "余烬" 抢占 "余烬A"
    chars.sort_by_key(|c| std::cmp::Reverse(c.len()));

    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);

        // ===== 情况 1：拖入的是文件夹 =====
        if path.is_dir() {
            let dir_name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if dir_name.is_empty() {
                continue;
            }

            // 1a. 拖入的就是某个已有角色目录 → 其下每个子文件夹各算一个 mod
            if chars.iter().any(|c| c == &dir_name) {
                if let Ok(entries) = fs::read_dir(path) {
                    let mut subs: Vec<_> = entries
                        .flatten()
                        .filter(|e| e.path().is_dir())
                        .filter(|e| {
                            let n = e.file_name().to_string_lossy().to_lowercase();
                            !n.starts_with("disabled")
                        })
                        .collect();
                    subs.sort_by_key(|e| e.file_name());
                    for e in subs {
                        let sub = e.path();
                        out.push(ImportCandidate {
                            path: sub.to_string_lossy().to_string(),
                            name: e.file_name().to_string_lossy().to_string(),
                            character: dir_name.clone(),
                            size: dir_size(&sub),
                        });
                    }
                }
                continue;
            }

            // 1b. 普通 mod 文件夹 → 按文件夹名匹配角色
            let matched = chars
                .iter()
                .find(|c| dir_name.contains(c.as_str()))
                .cloned()
                .unwrap_or_default();
            out.push(ImportCandidate {
                path: p.clone(),
                name: dir_name,
                character: matched,
                size: dir_size(path),
            });
            continue;
        }

        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !matches!(ext.as_str(), "zip" | "rar" | "7z") {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // 自动匹配：压缩包名 + 内部一级目录名 包含角色名
        let char_dir_hint = archive_top_dir(path, &ext).unwrap_or_default();
        let haystack = format!("{stem} {char_dir_hint}");
        let matched = chars
            .iter()
            .find(|c| haystack.contains(c.as_str()))
            .cloned()
            .unwrap_or_default();

        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        out.push(ImportCandidate {
            path: p.clone(),
            name: stem,
            character: matched,
            size,
        });
    }
    out
}

/// 递归统计文件夹大小（读不到的条目忽略）
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

/// 读取压缩包内部的一级目录名（用于辅助匹配角色）
fn archive_top_dir(archive: &Path, ext: &str) -> Option<String> {
    match ext {
        "zip" => {
            let file = fs::File::open(archive).ok()?;
            let mut z = zip::ZipArchive::new(file).ok()?;
            for i in 0..z.len().min(30) {
                let e = z.by_index(i).ok()?;
                let name = e.name().to_string();
                if let Some(first) = name.split(['/', '\\']).next() {
                    if !first.is_empty() && !first.contains('.') {
                        return Some(first.to_string());
                    }
                }
            }
            None
        }
        // rar/7z 读取成本高，跳过（仅用文件名匹配）
        _ => None,
    }
}

/// 导入一个 mod 压缩包（可指定 mod 名称，name 为空则用压缩包文件名）。
/// - mods_path: Mods 根目录
/// - archive: 压缩包路径（.zip/.rar/.7z）
/// - character: 目标角色名
pub fn import_mod(mods_path: &str, archive: &str, character: &str) -> Result<String, String> {
    import_mod_named(mods_path, archive, character, "")
}

/// 导入 mod，可自定义目标文件夹名（name 为空则用压缩包文件名）
pub fn import_mod_named(
    mods_path: &str,
    archive: &str,
    character: &str,
    name: &str,
) -> Result<String, String> {
    let src_path = Path::new(archive);
    if !src_path.exists() {
        return Err(format!("源文件不存在: {archive}"));
    }

    let char_dir = Path::new(mods_path).join(character);
    fs::create_dir_all(&char_dir).map_err(|e| format!("创建角色目录失败: {e}"))?;

    // 源可能是压缩包，也可能是一个文件夹
    let is_dir_source = src_path.is_dir();
    if !is_dir_source && !is_archive(src_path) {
        return Err("仅支持 zip / rar / 7z 压缩包或文件夹".into());
    }

    // mod 名：优先用自定义名，否则文件夹用目录名、压缩包用文件名去扩展
    let mod_name = if name.trim().is_empty() {
        let base = if is_dir_source {
            src_path.file_name()
        } else {
            src_path.file_stem()
        };
        base.map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "mod".into())
    } else {
        name.trim().to_string()
    };

    // 目标：外层目录（保持「外层 + 内层」结构，与原管理器一致）
    let target = char_dir.join(&mod_name);
    if target.exists() {
        return Err(format!("同名 mod 已存在: {character}/{mod_name}"));
    }

    // 文件夹来源：直接复制内容到目标目录，不解压
    if is_dir_source {
        let src_abs = fs::canonicalize(src_path).unwrap_or_else(|_| src_path.to_path_buf());
        let char_dir_abs = fs::canonicalize(&char_dir).unwrap_or_else(|_| char_dir.clone());
        if char_dir_abs.join(&mod_name).starts_with(&src_abs) {
            return Err("目标位置位于源文件夹内部，已取消导入".into());
        }
        return copy_dir_all(src_path, &target)
            .map(|_| mod_name)
            .map_err(|e| format!("复制文件夹失败: {e}"));
    }

    let tmp = temp_extract_dir("efmi-import")?;
    let result = extract_archive(src_path, &tmp);

    match result {
        Ok(()) => {
            clean_junk(&tmp);
            // 解压内容整体作为外层，保持压缩包原有目录层次不动
            // （有的 mod 就是多层目录、多个 ini，不做任何折叠）
            move_dir_all(&tmp, &target).map_err(|e| format!("移动到目标失败: {e}"))?;
            Ok(mod_name)
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

/// 跨盘安全的目录移动：先尝试 rename，失败则复制后删除
pub(crate) fn move_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_dir_all(src, dst)?;
    let _ = fs::remove_dir_all(src);
    Ok(())
}

pub(crate) fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {e}"))?;
    let entries = fs::read_dir(src).map_err(|e| format!("读取目录失败: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
        let p = entry.path();
        let dest = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir_all(&p, &dest)?;
        } else {
            fs::copy(&p, &dest).map_err(|e| format!("复制文件失败 {p:?}: {e}"))?;
        }
    }
    Ok(())
}

/// 解压任意支持的压缩包到 dest（zip / 7z / rar）
pub(crate) fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let ext = archive
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "zip" => extract_zip(archive, dest),
        "7z" => extract_7z(archive, dest),
        "rar" => extract_rar(archive, dest),
        _ => Err("仅支持 zip / rar / 7z 压缩包".into()),
    }
}

/// 是否是可解的压缩包扩展名
pub(crate) fn is_archive(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(ext.as_str(), "zip" | "7z" | "rar")
}

/// 带密码的压缩包统一提示：不提供解密能力，引导用户自行解压后拖文件夹
const PASSWORD_HINT: &str =
    "该压缩包带密码，本应用不支持解密。请先用解压软件把它解开，再把解压出来的文件夹拖进来导入。";

/// 解压 zip（用 zip crate）
fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| format!("打开压缩包失败: {e}"))?;
    let mut z = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {e}"))?;
    for i in 0..z.len() {
        let mut entry = z.by_index(i).map_err(map_zip_err)?;
        // 加密条目：直接拒绝，不做解密尝试
        if entry.encrypted() {
            return Err(PASSWORD_HINT.into());
        }
        let out_path = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| format!("创建目录失败: {e}"))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
            }
            let mut out = fs::File::create(&out_path).map_err(|e| format!("创建文件失败: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("解压写入失败: {e}"))?;
        }
    }
    Ok(())
}

/// 解压 7z（纯 Rust 实现，sevenz-rust2）
fn extract_7z(archive: &Path, dest: &Path) -> Result<(), String> {
    sevenz_rust2::decompress_file(
        archive.to_string_lossy().as_ref(),
        dest.to_string_lossy().as_ref(),
    )
    .map_err(|e| match e {
        sevenz_rust2::Error::PasswordRequired => PASSWORD_HINT.to_string(),
        other => format!("解压 7z 失败: {other}"),
    })
}

/// zip 错误统一映射：密码相关转成友好提示
fn map_zip_err(e: zip::result::ZipError) -> String {
    use zip::result::ZipError as Z;
    match &e {
        Z::UnsupportedArchive(msg) if *msg == Z::PASSWORD_REQUIRED => PASSWORD_HINT.into(),
        Z::InvalidPassword => PASSWORD_HINT.into(),
        _ => format!("读取条目失败: {e}"),
    }
}

/// 解压 rar（内置 UnRAR 源码静态编译，运行时不依赖系统的 7-Zip / Bandizip）
fn extract_rar(archive: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("创建目录失败: {e}"))?;
    let mut cursor = unrar::Archive::new(archive)
        .open_for_processing()
        .map_err(|e| format!("解析 rar 失败: {e}"))?;
    loop {
        let header = match cursor.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => {
                let msg = e.to_string();
                let lower = msg.to_lowercase();
                if lower.contains("password") || lower.contains("encrypt") {
                    return Err(PASSWORD_HINT.into());
                }
                return Err(format!("读取 rar 条目失败: {msg}"));
            }
        };
        cursor = header.extract_with_base(dest).map_err(|e| {
            let msg = e.to_string();
            let lower = msg.to_lowercase();
            if lower.contains("password") || lower.contains("encrypt") {
                PASSWORD_HINT.to_string()
            } else {
                format!("解压 rar 条目失败: {msg}")
            }
        })?;
    }
    Ok(())
}

/// 清理解压结果里的系统垃圾条目（macOS / Windows 生成的辅助文件）
fn clean_junk(dir: &Path) {
    const JUNK: &[&str] = &["__MACOSX", ".DS_Store", "Thumbs.db", "desktop.ini"];
    for name in JUNK {
        let p = dir.join(name);
        if p.is_dir() {
            let _ = fs::remove_dir_all(&p);
        } else if p.exists() {
            let _ = fs::remove_file(&p);
        }
    }
}

/// 生成临时解压目录
pub(crate) fn temp_extract_dir(prefix: &str) -> Result<PathBuf, String> {
    // 进程号 + 毫秒 + 单调序号：并发导入时不会撞名（并发只靠毫秒不够）
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        seq
    ));
    fs::create_dir_all(&tmp).map_err(|e| format!("创建临时目录失败: {e}"))?;
    Ok(tmp)
}

// ===== 压缩包落盘结构测试 =====
#[cfg(test)]
mod structure_tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files {
            zw.start_file(*name, opts).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }

    fn setup(tag: &str) -> (PathBuf, PathBuf) {
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("emm-struct-mods-{tag}-{pid}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("余烬")).unwrap();
        let work = std::env::temp_dir().join(format!("emm-struct-work-{tag}-{pid}"));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(&work).unwrap();
        (root, work)
    }

    fn mod_count(root: &Path) -> usize {
        let groups = crate::scan::scan_mods(&root.to_string_lossy()).unwrap();
        groups
            .iter()
            .find(|g| g.name == "余烬")
            .map(|g| g.mods.len())
            .unwrap_or(0)
    }

    /// 包根目录直接是 Meshes/Textures/mod.ini（没有外层文件夹）：
    /// 应落地为 `角色/包名/...`，且被扫描识别为一个 mod
    #[test]
    fn zip_root_is_flat() {
        let (root, work) = setup("flat");
        let zp = work.join("叶苔TS.zip");
        make_zip(
            &zp,
            &[
                ("mod.ini", b"[TextureOverrideX]\nhash=abcd\n"),
                ("Meshes/ab12.buf", b"mesh"),
                ("Textures/ab12.dds", b"tex"),
            ],
        );

        let n = import_mod_named(&root.to_string_lossy(), &zp.to_string_lossy(), "余烬", "叶苔TS")
            .expect("导入失败");
        assert_eq!(n, "叶苔TS");

        let mod_dir = root.join("余烬").join("叶苔TS");
        for p in ["mod.ini", "Meshes/ab12.buf", "Textures/ab12.dds"] {
            assert!(mod_dir.join(p).exists(), "缺少 {p}");
        }
        assert_eq!(mod_count(&root), 1, "扫描未识别到该 mod");

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&work);
    }

    /// 包内自带文件夹（甚至是多层、多个 ini）：原样保留，不做任何折叠
    #[test]
    fn zip_with_inner_folder_is_preserved() {
        let (root, work) = setup("inner");
        let zp = work.join("inner.zip");
        make_zip(
            &zp,
            &[
                ("Outer/Inner/mod.ini", b"[TextureOverrideX]\n"),
                ("Outer/Inner/Sub/extra.ini", b"[TextureOverrideY]\n"),
                ("Outer/Inner/Meshes/a.buf", b"mesh"),
            ],
        );

        import_mod_named(&root.to_string_lossy(), &zp.to_string_lossy(), "余烬", "inner")
            .expect("导入失败");

        let mod_dir = root.join("余烬").join("inner");
        assert!(
            mod_dir.join("Outer/Inner/mod.ini").exists(),
            "原始目录层次应完整保留"
        );
        assert!(
            mod_dir.join("Outer/Inner/Sub/extra.ini").exists(),
            "多层目录与多个 ini 都应保留"
        );
        assert!(!mod_dir.join("mod.ini").exists(), "不应把内层内容提上来");

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&work);
    }
}

//! 画廊：收集某个角色下所有 Mod 的可渲染图片（png/jpg/jpeg/webp/gif）。
//!
//! 规则（与 QAQM 对齐）：
//! - 每个 Mod 目录递归收集图片，但 `preview.*`（mod 根目录）排最前
//! - 跳过 qaqm 缓存目录，避免混入状态文件
//! - DISABLED_ 前缀的 Mod 也纳入展示（带 enabled 标记）

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct GalleryMod {
    /// mod 显示名（去掉 DISABLED_ 前缀）
    pub name: String,
    /// 磁盘上的实际文件夹名
    pub folder_name: String,
    /// 是否启用
    pub enabled: bool,
    /// 相对 mods_path 的路径（角色/mod名）
    pub rel_path: String,
    /// 图片绝对路径列表（已排序：preview 优先）
    pub images: Vec<String>,
}

const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

fn is_image_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    IMAGE_EXTS.iter().any(|e| lower.ends_with(&format!(".{e}")))
}

/// 是否要跳过的目录（qaqm 缓存、EFMI 特殊目录）
fn is_skipped_dir(name: &str) -> bool {
    let n = name.to_lowercase();
    n == "qaqm" || n.starts_with("disabled - ") || n.starts_with("do not modify")
}

/// 递归收集目录下的图片文件（不跟随符号链接，深度不限）
fn collect_images(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = e.file_name().to_string_lossy().to_string();
            if is_skipped_dir(&name) {
                continue;
            }
            subdirs.push(p);
        } else if p.is_file() {
            let name = e.file_name().to_string_lossy().to_string();
            if is_image_file(&name) {
                files.push(p);
            }
        }
    }
    files.sort();
    out.extend(files);
    subdirs.sort();
    for d in subdirs {
        collect_images(&d, out);
    }
}

/// 把「preview.* 在最前」的排序应用到已收集的图片列表
fn sort_preview_first(images: &mut Vec<PathBuf>, mod_dir: &Path) {
    let preview = ["preview.png", "preview.jpg", "preview.jpeg", "preview.webp", "preview.gif"]
        .iter()
        .map(|n| mod_dir.join(n))
        .find(|p| p.is_file());
    let Some(preview) = preview else { return };
    images.retain(|p| p != &preview);
    images.insert(0, preview);
}

/// 收集某个角色下所有 Mod 的图片
pub fn collect_gallery(mods_path: &str, character: &str) -> Result<Vec<GalleryMod>, String> {
    let char_dir = Path::new(mods_path).join(character);
    if !char_dir.is_dir() {
        return Err(format!("角色目录不存在: {}", char_dir.display()));
    }

    let entries = fs::read_dir(&char_dir).map_err(|e| format!("读取角色目录失败: {e}"))?;
    let mut mod_dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    mod_dirs.sort_by_key(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));

    let mut result = Vec::new();
    for mod_dir in mod_dirs {
        let folder_name = match mod_dir.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        let name = folder_name
            .strip_prefix("DISABLED_")
            .unwrap_or(&folder_name)
            .to_string();
        let enabled = !folder_name.starts_with("DISABLED_");

        let mut images: Vec<PathBuf> = Vec::new();
        collect_images(&mod_dir, &mut images);
        sort_preview_first(&mut images, &mod_dir);

        if images.is_empty() {
            continue;
        }

        result.push(GalleryMod {
            name,
            folder_name: folder_name.clone(),
            enabled,
            rel_path: format!("{character}/{folder_name}"),
            images: images
                .into_iter()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect(),
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_gallery_collect() {
        let tmp = std::env::temp_dir().join(format!("efmi_gallery_test_{}", std::process::id()));
        let mod_dir = tmp.join("角色A").join("测试Mod");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::create_dir_all(mod_dir.join("k")).unwrap();
        fs::write(mod_dir.join("preview.png"), b"x").unwrap();
        fs::write(mod_dir.join("k").join("k.png"), b"x").unwrap();
        fs::write(mod_dir.join("k").join("panel_icon.png"), b"x").unwrap();
        fs::write(mod_dir.join("note.txt"), b"x").unwrap();

        let res = collect_gallery(tmp.to_str().unwrap(), "角色A").unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "测试Mod");
        assert!(res[0].enabled);
        // preview 排最前
        assert!(res[0].images[0].ends_with("preview.png"));
        assert_eq!(res[0].images.len(), 3);

        // DISABLED 前缀
        let disabled_dir = tmp.join("角色A").join("DISABLED_另一个Mod");
        fs::create_dir_all(&disabled_dir).unwrap();
        fs::write(disabled_dir.join("a.jpg"), b"x").unwrap();
        let res2 = collect_gallery(tmp.to_str().unwrap(), "角色A").unwrap();
        assert_eq!(res2.len(), 2);
        let d = res2.iter().find(|m| m.name == "另一个Mod").unwrap();
        assert!(!d.enabled);
        assert_eq!(d.images.len(), 1);

        fs::remove_dir_all(&tmp).ok();
    }
}

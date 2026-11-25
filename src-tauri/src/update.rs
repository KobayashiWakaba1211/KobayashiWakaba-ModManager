//! 应用更新检测：查询本仓库 GitHub Releases 的最新版本，
//! 与当前程序版本比较，供「关于」页的检查更新功能使用。

use serde::Serialize;
use serde_json::Value;

const OWNER: &str = "KobayashiWakaba1211";
const REPO: &str = "KobayashiWakaba-ModManager";
const RELEASES_API: &str = "https://api.github.com/repos/KobayashiWakaba1211/KobayashiWakaba-ModManager/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/KobayashiWakaba1211/KobayashiWakaba-ModManager/releases/latest";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// 当前程序版本（来自 Cargo.toml）
    pub current_version: String,
    /// 最新 Release 的版本（tag 去掉 v 前缀）
    pub latest_version: String,
    pub has_update: bool,
    /// Releases 页面（浏览器打开）
    pub release_url: String,
    /// 安装包直链（如无安装包附件则回退 Releases 页）
    pub download_url: String,
    /// 发布日期（yyyy-mm-dd）
    pub published_at: String,
    /// Release 说明（原文，超长截断）
    pub notes: String,
}

/// "v0.1.2" / "0.1.2" / "0.1.2-beta" → (0, 1, 2)；解析失败返回 None
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let t = s.trim().trim_start_matches(['v', 'V']);
    let core = t.split(['-', '+']).next().unwrap_or(t);
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// 最新版是否比当前版新
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// 从 assets 里找 Windows 安装包（文件名以 -setup.exe 结尾，大小写不敏感）
fn find_setup_asset(v: &Value) -> Option<String> {
    v.get("assets")?.as_array()?.iter().find_map(|a| {
        let name = a.get("name")?.as_str()?;
        if name.to_ascii_lowercase().ends_with("-setup.exe") {
            a.get("browser_download_url")?.as_str().map(String::from)
        } else {
            None
        }
    })
}

/// 查询最新 Release 并比较
pub async fn check() -> Result<UpdateInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建网络客户端失败: {e}"))?;
    let resp = client
        .get(RELEASES_API)
        .header("User-Agent", format!("{REPO}-update-check"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("连接 GitHub 失败: {e}"))?;
    // GitHub 匿名 API 每 IP 每小时 60 次；共享网络出口下容易撞限流，给友好提示
    if resp.status() == reqwest::StatusCode::FORBIDDEN || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("GitHub 接口限流（同一网络下请求太多），请过半小时再试".to_string());
    }
    let resp = resp
        .error_for_status()
        .map_err(|e| format!("GitHub 返回错误: {e}"))?;
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {e}"))?;

    let tag = v
        .get("tag_name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if tag.is_empty() {
        return Err("Release 数据里没有版本号".to_string());
    }
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    let current = env!("CARGO_PKG_VERSION").to_string();

    let published_at = v
        .get("published_at")
        .and_then(|x| x.as_str())
        .map(|s| s.get(0..10).unwrap_or(s).to_string())
        .unwrap_or_default();
    let notes = v
        .get("body")
        .and_then(|x| x.as_str())
        .map(|s| {
            if s.chars().count() > 1500 {
                format!("{}…", s.chars().take(1500).collect::<String>())
            } else {
                s.to_string()
            }
        })
        .unwrap_or_default();

    let download_url = find_setup_asset(&v).unwrap_or_else(|| RELEASES_PAGE.to_string());

    Ok(UpdateInfo {
        has_update: is_newer(&current, &latest),
        current_version: current,
        latest_version: latest,
        release_url: RELEASES_PAGE.to_string(),
        download_url,
        published_at,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_and_compare() {
        assert_eq!(parse_version("v0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_version("0.1.10"), Some((0, 1, 10)));
        assert_eq!(parse_version("V1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("abc"), None);
        assert!(is_newer("0.1.1", "v0.1.2"));
        assert!(is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.2", "v0.1.1"));
        assert!(is_newer("0.1.2", "0.1.10")); // 逐段比较，不是字符串比较
    }

    #[test]
    fn finds_setup_asset() {
        let v = json!({
            "tag_name": "v0.1.2",
            "assets": [
                { "name": "KobayashiWakaba-ModManager-v0.1.2-source.zip", "browser_download_url": "https://x/src.zip" },
                { "name": "KobayashiWakaba-ModManager_0.1.2_x64-setup.exe", "browser_download_url": "https://x/setup.exe" },
                { "name": "README.md", "browser_download_url": "https://x/readme" }
            ]
        });
        assert_eq!(find_setup_asset(&v).as_deref(), Some("https://x/setup.exe"));
        // 大小写不敏感
        let v2 = json!({ "assets": [{ "name": "App-Setup.EXE", "browser_download_url": "https://x/s.exe" }] });
        assert_eq!(find_setup_asset(&v2).as_deref(), Some("https://x/s.exe"));
        // 没有安装包附件时返回 None（前端回退到 Releases 页）
        let v3 = json!({ "assets": [{ "name": "src.zip", "browser_download_url": "https://x" }] });
        assert_eq!(find_setup_asset(&v3), None);
        let v4 = json!({});
        assert_eq!(find_setup_asset(&v4), None);
    }
}

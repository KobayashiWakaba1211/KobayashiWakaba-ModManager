//! 启动游戏：复刻 QAQ Manager 的 launch-game 逻辑
//!  - 原版启动：start "" /D "<游戏目录>" "<游戏 exe>"
//!  - EFMI/XXMI/SRMI/WWMI/ZZMI：start "" /D "<启动器目录>" "<XXMI Launcher.exe>" --nogui --xxmi <MODE>
//!  - XXMI（带 GUI）：start "" /D "<启动器目录>" "<XXMI Launcher.exe>"
//!  - GIML（原神）：直接运行 GIML.exe 加壳启动器

use crate::config::{AppConfig, Game};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 每个内置游戏对应的导入器名（用于 --xxmi 参数）
pub fn importer_name(game_id: &str) -> &'static str {
    match game_id {
        "endfield" => "EFMI",
        "starrail" => "SRMI",
        "wuthering" => "WWMI",
        "zenless" => "ZZMI",
        "GenshinImpact" => "GIMI",
        _ => "",
    }
}

/// 该游戏支持的启动模式列表（复刻 QAQM 的 supportedLaunchModes）
/// 原神用 GIML（加壳启动器）替代原来的 GIMI 注入模式。
pub fn supported_modes(game_id: &str) -> Vec<String> {
    if game_id == "GenshinImpact" {
        return vec!["GIML".to_string(), "XXMI".to_string()];
    }
    let imp = importer_name(game_id);
    if imp.is_empty() {
        vec!["XXMI".to_string()]
    } else {
        vec![imp.to_string(), "XXMI".to_string()]
    }
}

/// 在常见位置找 GIML.exe：桌面 / 下载目录下的 GIML* 文件夹，或直接放在这两个目录里
/// 多个候选时选「修改时间最新」的那个（用户常同时留着旧版本和新版本文件夹）
fn find_giml_exe() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        roots.push(home.join("Desktop"));
        roots.push(home.join("Downloads"));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        // 直接放在 Desktop / Downloads
        let direct = root.join("GIML.exe");
        if direct.is_file() {
            candidates.push(direct);
        }
        // 一层子目录里的 GIML.exe（如 "GIML V3.0.0 帧数解锁版\GIML.exe"）
        if let Ok(rd) = fs::read_dir(&root) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_uppercase();
                if !name.starts_with("GIML") {
                    continue;
                }
                let exe = p.join("GIML.exe");
                if exe.is_file() {
                    candidates.push(exe);
                }
            }
        }
    }
    candidates
        .into_iter()
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

/// 解析 GIML.exe 的有效路径（配置优先 → 桌面/下载目录自动探测）
pub fn resolved_giml_path(game: &Game) -> String {
    if !game.giml_path.trim().is_empty() {
        return game.giml_path.trim().to_string();
    }
    find_giml_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 默认 XXMI Launcher.exe 位置（%APPDATA%\XXMI Launcher\Resources\Bin\XXMI Launcher.exe）
fn default_launcher() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").map(PathBuf::from)?;
    let p = base
        .join("XXMI Launcher")
        .join("Resources")
        .join("Bin")
        .join("XXMI Launcher.exe");
    if p.is_file() { Some(p) } else { None }
}

/// 读 XXMI Launcher Config.json，取某个导入器的 game_folder / launcher_folder
/// 返回 (game_folder, launcher_folder)
fn read_xxmi_game_folder(importer: &str) -> Option<(String, String)> {
    let base = std::env::var_os("APPDATA").map(PathBuf::from)?;
    let cfg = base.join("XXMI Launcher").join("XXMI Launcher Config.json");
    let raw = fs::read_to_string(cfg).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let im = json.get("Importers")?.get(importer)?.get("Importer")?;
    let get = |k: &str| -> String {
        im.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Some((get("game_folder"), get("launcher_folder")))
}

/// 从 game_folder 找游戏 exe（复刻常见 exe 名）
fn find_game_exe(game_folder: &str) -> Option<PathBuf> {
    if game_folder.is_empty() {
        return None;
    }
    let dir = Path::new(game_folder);
    if !dir.is_dir() {
        return None;
    }
    // 优先精确 exe 名
    const EXE_NAMES: &[&str] = &[
        "Endfield.exe",
        "Arknights.exe",
        "StarRail.exe",
        "ZenlessZoneZero.exe",
        "Client-Win64-Shipping.exe",
        "GenshinImpact.exe",
        "YuanShen.exe",
    ];
    for name in EXE_NAMES {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    // 兜底：目录里第一个 exe
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("exe") && p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 解析当前激活游戏的有效启动路径（配置优先 → XXMI 配置推断 → 默认安装位置）
/// 返回 (game_path, launcher_path)
pub fn resolved_paths(cfg: &AppConfig, game: &Game) -> (String, String) {
    let imp = importer_name(&game.id);

    // 启动器路径：全局配置 > 该游戏遗留值 > 默认安装位置
    let launcher = if !cfg.xxmi_launcher_path.trim().is_empty() {
        cfg.xxmi_launcher_path.trim().to_string()
    } else if !game.launcher_path.trim().is_empty() {
        game.launcher_path.trim().to_string()
    } else {
        default_launcher()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    // 游戏路径：用户配置 > XXMI 配置里的 game_folder（找 exe）> launcher_folder
    let game_path = if !game.game_path.trim().is_empty() {
        game.game_path.trim().to_string()
    } else if !imp.is_empty() {
        if let Some((folder, launcher_folder)) = read_xxmi_game_folder(imp) {
            if let Some(exe) = find_game_exe(&folder) {
                exe.to_string_lossy().to_string()
            } else if !launcher_folder.trim().is_empty() {
                launcher_folder
            } else {
                folder
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let _ = cfg; // 保留参数以便未来扩展
    (game_path, launcher)
}

/// 把附加参数字符串拆成 argv（支持 "..." 和 '...' 包裹的带空格参数），复刻 QAQM 的 parseLaunchArgs
fn parse_launch_args(text: &str) -> Vec<String> {
    let re = regex_lite_arg_split(text);
    re
}

/// 轻量参数拆分：引号内不分词，去掉成对引号
fn regex_lite_arg_split(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for ch in text.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    started = true;
                }
                c if c.is_whitespace() => {
                    if started {
                        out.push(std::mem::take(&mut cur));
                        started = false;
                    }
                }
                c => {
                    cur.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// 启动 exe —— 复刻 QAQM launchWindowsExecutableViaStart 的效果（工作目录 + 参数 + 异步脱离）
///
/// 注意：不通过 `cmd /C start ...` 拼接字符串。原因是 Rust 的 Command 会把参数里的引号
/// 转义为 `\"`，而 cmd.exe 不识别该转义，导致带空格路径被解析成 `\\...` 并报「找不到文件」。
/// 直接 spawn exe 并设置 current_dir 完全等价，且避免引号地狱。
fn start_via_windows(exe: &str, args: &str) -> Result<(), String> {
    if exe.trim().is_empty() {
        return Err("路径为空".to_string());
    }
    let p = Path::new(exe);
    if !p.is_file() {
        return Err(format!("程序不存在: {exe}"));
    }
    let dir = p.parent().unwrap_or_else(|| Path::new(""));

    let argv = parse_launch_args(args);
    let mut command = Command::new(exe);
    command.current_dir(dir);
    if !argv.is_empty() {
        command.args(&argv);
    }

    #[cfg(windows)]
    {
        // 让子进程独立运行：DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        // 这样关闭管理器窗口也不会连带终止游戏/启动器。
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    match command.spawn() {
        Ok(_) => Ok(()),
        // ERROR_ELEVATION_REQUIRED(740)：目标 exe 的清单要求管理员权限（如 XXMI Launcher）。
        // CreateProcess 无法跨权限拉起，改走 ShellExecute，由系统弹 UAC 授权后启动。
        // 管理器自身保持普通权限（否则外部文件无法拖入窗口）。
        Err(e) if e.raw_os_error() == Some(740) => {
            #[cfg(windows)]
            {
                start_via_shell_execute(exe, &argv, dir).map_err(|e2| format!("{e}；改用提权启动仍失败：{e2}"))
            }
            #[cfg(not(windows))]
            {
                Err(format!("启动失败: {e}"))
            }
        }
        Err(e) => Err(format!("启动失败: {e}")),
    }
}

/// 把参数列表拼成 ShellExecute 用的命令行字符串（含空白的参数加引号）
#[cfg(any(windows, test))]
fn join_shell_args(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() || a.split_whitespace().count() > 1 {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 通过 ShellExecuteExW 启动。目标需要提权时，系统会弹出 UAC 授权框。
#[cfg(windows)]
fn start_via_shell_execute(exe: &str, argv: &[String], dir: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GetLastError, HWND};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};

    const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
    const SEE_MASK_FLAG_NO_UI: u32 = 0x0000_0400;
    const SW_SHOWNORMAL: i32 = 1;
    const ERROR_CANCELLED: u32 = 1223;

    let file: Vec<u16> = exe.encode_utf16().chain(std::iter::once(0)).collect();
    let params_str = join_shell_args(argv);
    let params: Vec<u16> = if params_str.is_empty() {
        Vec::new()
    } else {
        params_str.encode_utf16().chain(std::iter::once(0)).collect()
    };
    let dir_str = dir.to_string_lossy().to_string();
    let directory: Vec<u16> = if dir_str.is_empty() {
        Vec::new()
    } else {
        dir_str.encode_utf16().chain(std::iter::once(0)).collect()
    };

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.hwnd = HWND::default();
    info.lpFile = file.as_ptr() as *mut u16;
    info.lpParameters = if params.is_empty() {
        std::ptr::null_mut()
    } else {
        params.as_ptr() as *mut u16
    };
    info.lpDirectory = if directory.is_empty() {
        std::ptr::null_mut()
    } else {
        directory.as_ptr() as *mut u16
    };
    info.nShow = SW_SHOWNORMAL;

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_CANCELLED {
            return Err("已取消管理员授权".to_string());
        }
        return Err(format!("系统拒绝启动 (os error {code})"));
    }
    Ok(())
}

/// 执行启动（供 lib.rs 的 Tauri command 调用）
/// launch_mode: Some(mode) 临时覆盖；None = 用配置里的模式
pub fn launch(cfg: &AppConfig, game: &Game, launch_mode: Option<&str>) -> Result<(), String> {
    let mode = launch_mode
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| game.launch_mode.trim().to_string());
    // 空模式 = 原版
    let mode = if mode.is_empty() { "VANILLA".to_string() } else { mode.to_uppercase() };
    // 兼容旧配置：原神原来的 GIMI 注入模式已换成 GIML 加壳启动器
    let mode = if game.id == "GenshinImpact" && mode == "GIMI" {
        "GIML".to_string()
    } else {
        mode
    };

    let extra = game.launch_args.trim();
    let (game_path, launcher_path) = resolved_paths(cfg, game);

    match mode.as_str() {
        "VANILLA" => {
            if game_path.is_empty() || !Path::new(&game_path).is_file() {
                return Err(format!(
                    "游戏路径未设置或无效：{game_path}\n请在「系统设置 → 当前游戏 → 游戏程序路径」中选择游戏 exe 或官方启动器。"
                ));
            }
            start_via_windows(&game_path, extra)
        }
        "GIML" => {
            let giml = resolved_giml_path(game);
            if giml.is_empty() || !Path::new(&giml).is_file() {
                return Err(format!(
                    "未找到 GIML.exe：{giml}\n请在「系统设置 → 启动设置 → GIML.exe 路径」中选择 GIML.exe（例如桌面的 GIML V3.0.0 帧数解锁版\\GIML.exe）。"
                ));
            }
            // GIML 是加壳启动器：直接运行，参数原样透传
            start_via_windows(&giml, extra)
        }
        "XXMI" => {
            if launcher_path.is_empty() || !Path::new(&launcher_path).is_file() {
                return Err(format!(
                    "未找到 XXMI Launcher.exe：{launcher_path}\n请在「系统设置 → XXMI Launcher（全局）」中选择，或重新安装 XXMI Launcher。"
                ));
            }
            // XXMI 模式：直接启动启动器（带 GUI）
            start_via_windows(&launcher_path, extra)
        }
        "EFMI" | "SRMI" | "WWMI" | "ZZMI" | "GIMI" => {
            // 校验模式与游戏匹配（复刻 QAQM 的 game id 检查）
            let imp = importer_name(&game.id);
            if !imp.is_empty() && mode != imp {
                return Err(format!("当前游戏「{}」不支持 {mode} 启动模式。", game.name));
            }
            if launcher_path.is_empty() || !Path::new(&launcher_path).is_file() {
                return Err(format!(
                    "未找到 XXMI Launcher.exe：{launcher_path}\n请在「系统设置 → XXMI Launcher（全局）」中选择，或重新安装 XXMI Launcher。"
                ));
            }
            // 快捷启动：--nogui --xxmi <MODE>（复刻 QAQM 原样命令）
            let args = if extra.is_empty() {
                format!("--nogui --xxmi {mode}")
            } else {
                format!("--nogui --xxmi {mode} {extra}")
            };
            start_via_windows(&launcher_path, &args)
        }
        _ => Err(format!("未知启动模式: {mode}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importer_mapping() {
        assert_eq!(importer_name("endfield"), "EFMI");
        assert_eq!(importer_name("starrail"), "SRMI");
        assert_eq!(importer_name("wuthering"), "WWMI");
        assert_eq!(importer_name("zenless"), "ZZMI");
        assert_eq!(importer_name("GenshinImpact"), "GIMI");
        assert_eq!(importer_name("custom"), "");
    }

    #[test]
    fn test_supported_modes() {
        assert_eq!(supported_modes("endfield"), vec!["EFMI", "XXMI"]);
        assert_eq!(supported_modes("GenshinImpact"), vec!["GIML", "XXMI"]);
        assert_eq!(supported_modes("custom"), vec!["XXMI"]);
    }

    #[test]
    fn shell_args_roundtrip() {
        // 无空白的参数原样；含空白的加引号；空参数也加引号（否则会丢失）
        assert_eq!(join_shell_args(&[]), "");
        assert_eq!(
            join_shell_args(&["--nogui".into(), "--xxmi".into(), "EFMI".into()]),
            "--nogui --xxmi EFMI"
        );
        assert_eq!(
            join_shell_args(&["-dx11".into(), "C:\\My Mods\\a b.ini".into(), String::new()]),
            "-dx11 \"C:\\My Mods\\a b.ini\" \"\""
        );
        // 与 parse_launch_args 互逆：拼出来的字符串再解析应得到同一组参数
        let argv = parse_launch_args("--nogui --xxmi EFMI -screen-width 1920");
        assert_eq!(parse_launch_args(&join_shell_args(&argv)), argv);
    }

    #[test]
    fn parse_args_simple() {
        assert_eq!(parse_launch_args("--nogui --xxmi EFMI"), vec!["--nogui", "--xxmi", "EFMI"]);
    }

    #[test]
    fn parse_args_with_quotes() {
        assert_eq!(
            parse_launch_args("-dx11 -path \"C:\\Program Files\\Game\""),
            vec!["-dx11", "-path", "C:\\Program Files\\Game"]
        );
        assert_eq!(
            parse_launch_args("--foo='a b' --bar"),
            vec!["--foo=a b", "--bar"]
        );
    }

    #[test]
    fn parse_args_empty() {
        assert!(parse_launch_args("").is_empty());
        assert!(parse_launch_args("   ").is_empty());
    }
}

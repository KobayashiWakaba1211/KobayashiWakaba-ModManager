use std::sync::{Mutex, MutexGuard, OnceLock};
use tauri::{Emitter, Manager};

fn show_overlay(app: &tauri::AppHandle) -> Result<(), String> {
    let window = match app.get_webview_window("overlay") {
        Some(window) => window,
        None => tauri::WebviewWindowBuilder::new(
            app,
            "overlay",
            tauri::WebviewUrl::App("index.html?window=overlay".into()),
        )
        .title("悬浮窗")
        .inner_size(560.0, 680.0)
        .min_inner_size(320.0, 240.0)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| format!("创建悬浮窗失败: {e}"))?,
    };
    window.show().map_err(|e| format!("显示悬浮窗失败: {e}"))?;
    let focus_result = window
        .set_focus()
        .map_err(|e| format!("聚焦悬浮窗失败: {e}"));
    if let Err(e) = app.emit_to("overlay", "overlay-refresh", ()) {
        eprintln!("发送悬浮窗刷新通知失败: {e}");
    }
    focus_result
}

#[tauri::command]
pub async fn open_overlay(app: tauri::AppHandle) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if sender.send(show_overlay(&handle)).is_err() {
            eprintln!("悬浮窗调用方已断开");
        }
    })
    .map_err(|e| format!("调度悬浮窗失败: {e}"))?;
    receiver
        .recv()
        .map_err(|e| format!("获取悬浮窗结果失败: {e}"))?
}

#[tauri::command]
pub fn hide_overlay(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("overlay") {
        window.hide().map_err(|e| format!("隐藏悬浮窗失败: {e}"))?;
    }
    Ok(())
}

// 由完成数据修改的窗口通知另一窗口重新读取持久状态，不传递可能过期的快照。
// 注意必须用 emit_to 定向发送：emit 是全局广播，会让发送方自己也收到事件，
// 造成两个窗口互相触发刷新的死循环。
#[tauri::command]
pub fn notify_overlay_state_changed(window: tauri::WebviewWindow) -> Result<(), String> {
    let target = match window.label() {
        "main" => "overlay",
        "overlay" => "main",
        _ => return Err("不支持的窗口".into()),
    };
    window
        .app_handle()
        .emit_to(target, "overlay-state-changed", ())
        .map_err(|e| format!("发送状态更新通知失败: {e}"))
}

// ===== 可配置全局热键 =====
//
// 设计：热键注册在独立线程（RegisterHotKey 绑定线程），该线程常驻消息循环；
// 修改请求串行处理，每个请求通过独立通道返回实际注册和持久化结果。

/// 解析后的热键（已带 MOD_NOREPEAT）
#[derive(Debug, Clone, Copy)]
struct HotkeySpec {
    mods: u32,
    vk: u32,
}

struct HotkeyState {
    #[cfg(windows)]
    request_lock: Mutex<()>,
    #[cfg(windows)]
    desired: Mutex<Option<HotkeyRequest>>,
    thread_id: Mutex<Option<u32>>,
    active: Mutex<Option<String>>,
}

#[cfg(windows)]
struct HotkeyRequest {
    spec: HotkeySpec,
    display: String,
    reply: std::sync::mpsc::SyncSender<Result<String, String>>,
}

static HOTKEY: OnceLock<HotkeyState> = OnceLock::new();
/// 自定义线程消息（WM_APP + 1）：通知热键线程重注册
const HOTKEY_CHANGE_MSG: u32 = 0x8000 + 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// 解析形如 "Ctrl+Alt+F" / "Shift+F10" 的快捷键描述。
/// 返回（规格，规范化显示名，如 "Ctrl+Alt+Shift+F"）。
fn parse_hotkey(input: &str) -> Result<(HotkeySpec, String), String> {
    #[cfg(windows)]
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    };
    #[cfg(not(windows))]
    const MOD_CONTROL: u32 = 0x0002;
    #[cfg(not(windows))]
    const MOD_ALT: u32 = 0x0001;
    #[cfg(not(windows))]
    const MOD_SHIFT: u32 = 0x0004;
    #[cfg(not(windows))]
    const MOD_WIN: u32 = 0x0008;
    #[cfg(not(windows))]
    const MOD_NOREPEAT: u32 = 0x4000;

    let tokens: Vec<&str> = input
        .split('+')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    let Some((last, mod_tokens)) = tokens.split_last() else {
        return Err("快捷键不能为空".into());
    };
    let mut mods = 0u32;
    for t in mod_tokens {
        mods |= match t.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" | "super" | "meta" => MOD_WIN,
            other => {
                return Err(format!(
                    "无法识别的修饰键: {other}（支持 Ctrl/Alt/Shift/Win）"
                ));
            }
        };
    }
    let key = last.to_ascii_uppercase();
    let is_fkey =
        key.len() >= 2 && key.starts_with('F') && key[1..].bytes().all(|b| b.is_ascii_digit());
    let vk: u32 = if key.len() == 1 {
        let c = key.chars().next().unwrap_or('\0');
        if c.is_ascii_alphanumeric() {
            c as u32
        } else {
            return Err(format!("不支持的主键: {last}（支持 A~Z、0~9、F1~F12）"));
        }
    } else if is_fkey {
        let n: u32 = key[1..]
            .parse()
            .map_err(|_| format!("不支持的主键: {last}"))?;
        if !(1..=12).contains(&n) {
            return Err(format!("F 键仅支持 F1~F12: {last}"));
        }
        0x70 + n - 1
    } else {
        return Err(format!("不支持的主键: {last}（支持 A~Z、0~9、F1~F12）"));
    };
    // 裸字母/数字会被注册为全局拦截、影响正常打字，要求搭配修饰键；F1~F12 可单独使用。
    if mods == 0 && !is_fkey {
        return Err("字母/数字主键必须搭配 Ctrl/Alt/Shift/Win（F1~F12 可单独使用）".into());
    }
    let mut display = String::new();
    for (flag, name) in [
        (MOD_CONTROL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
        (MOD_WIN, "Win"),
    ] {
        if mods & flag != 0 {
            if !display.is_empty() {
                display.push('+');
            }
            display.push_str(name);
        }
    }
    if !display.is_empty() {
        display.push('+');
    }
    if is_fkey {
        display.push_str(&format!("F{}", vk - 0x70 + 1));
    } else {
        display.push_str(&key);
    }
    Ok((
        HotkeySpec {
            mods: mods | MOD_NOREPEAT,
            vk,
        },
        display,
    ))
}

/// 把设置持久化到 config.json（load-modify-save，与主窗口尺寸保存存在理论上的并发窗口）
fn persist_hotkey(display: &str) -> Result<(), String> {
    let mut cfg = crate::config::load_config();
    cfg.overlay_hotkey = display.to_string();
    crate::config::save_config(&cfg)
}

/// 启动时初始化热键线程；configured 来自 config.json（空或非法时回退 Alt+F）。
#[cfg(windows)]
pub fn init_hotkey(app: tauri::AppHandle, configured: &str) {
    let state = HOTKEY.get_or_init(|| HotkeyState {
        request_lock: Mutex::new(()),
        desired: Mutex::new(None),
        thread_id: Mutex::new(None),
        active: Mutex::new(None),
    });
    let initial = match parse_hotkey(configured) {
        Ok(v) => v,
        Err(_) => match parse_hotkey("Alt+F") {
            Ok(v) => v,
            Err(e) => {
                eprintln!("默认快捷键解析失败: {e}");
                return;
            }
        },
    };
    let handle = app.clone();
    std::thread::spawn(move || hotkey_thread(handle, state, initial));
}

/// 非 Windows 平台：无全局热键，仅保留配置读写。
#[cfg(not(windows))]
pub fn init_hotkey(_app: tauri::AppHandle, _configured: &str) {}

#[cfg(windows)]
fn hotkey_thread(
    app: tauri::AppHandle,
    state: &'static HotkeyState,
    initial: (HotkeySpec, String),
) {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_HOTKEY,
    };

    const HOTKEY_ID: i32 = 1;
    let mut current = initial;
    unsafe {
        // SAFETY: 热键绑定到当前线程；MSG 为栈上合法结构；线程退出前注销热键。
        let mut registered = RegisterHotKey(
            std::ptr::null_mut(),
            HOTKEY_ID,
            current.0.mods,
            current.0.vk,
        ) != 0;
        if !registered {
            eprintln!(
                "注册全局快捷键 {} 失败: {}（可能已被其他程序占用，可在系统设置里更换）",
                current.1,
                std::io::Error::last_os_error()
            );
        }
        if registered {
            *lock(&state.active) = Some(current.1.clone());
        }
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        *lock(&state.thread_id) = Some(GetCurrentThreadId());
        loop {
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r <= 0 {
                if r < 0 {
                    eprintln!("读取快捷键消息失败: {}", std::io::Error::last_os_error());
                }
                break;
            }
            if msg.message == WM_HOTKEY && msg.wParam == HOTKEY_ID as usize {
                let toggle = app.clone();
                if let Err(e) = app.run_on_main_thread(move || {
                    let result = match toggle.get_webview_window("overlay") {
                        Some(window) => match window.is_visible() {
                            Ok(true) => window.hide().map_err(|e| e.to_string()),
                            Ok(false) => show_overlay(&toggle),
                            Err(e) => Err(e.to_string()),
                        },
                        None => show_overlay(&toggle),
                    };
                    if let Err(e) = result {
                        eprintln!("切换悬浮窗失败: {e}");
                    }
                }) {
                    eprintln!("调度悬浮窗失败: {e}");
                }
            } else if msg.message == HOTKEY_CHANGE_MSG {
                let desired = lock(&state.desired).take();
                if let Some(HotkeyRequest {
                    spec,
                    display,
                    reply,
                }) = desired
                {
                    let result = if registered
                        && current.0.mods == spec.mods
                        && current.0.vk == spec.vk
                    {
                        persist_hotkey(&display)
                            .map(|()| display.clone())
                            .map_err(|e| format!("快捷键 {display} 已注册，但保存配置失败: {e}"))
                    } else if registered && UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID) == 0 {
                        Err(format!(
                            "注销原快捷键 {} 失败: {}",
                            current.1,
                            std::io::Error::last_os_error()
                        ))
                    } else {
                        let was_registered = registered;
                        registered = false;
                        *lock(&state.active) = None;
                        if RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, spec.mods, spec.vk) != 0
                        {
                            registered = true;
                            current = (spec, display.clone());
                            *lock(&state.active) = Some(display.clone());
                            persist_hotkey(&display)
                                .map(|()| display.clone())
                                .map_err(|e| {
                                    format!("快捷键 {display} 已注册，但保存配置失败: {e}")
                                })
                        } else {
                            let error = std::io::Error::last_os_error();
                            if was_registered {
                                registered = RegisterHotKey(
                                    std::ptr::null_mut(),
                                    HOTKEY_ID,
                                    current.0.mods,
                                    current.0.vk,
                                ) != 0;
                                if registered {
                                    *lock(&state.active) = Some(current.1.clone());
                                }
                            }
                            Err(registration_error(
                                &display,
                                &current.1,
                                was_registered,
                                registered,
                                &error.to_string(),
                            ))
                        }
                    };
                    if reply.send(result).is_err() {
                        eprintln!("快捷键设置调用方已断开");
                    }
                }
            }
        }
        *lock(&state.thread_id) = None;
        if registered && UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID) == 0 {
            eprintln!("注销快捷键失败: {}", std::io::Error::last_os_error());
        }
        *lock(&state.active) = None;
        lock(&state.desired).take();
    }
}

#[cfg(any(windows, test))]
fn registration_error(
    display: &str,
    previous: &str,
    had_previous: bool,
    restored: bool,
    error: &str,
) -> String {
    let status = if restored {
        format!("已恢复原快捷键 {previous}")
    } else if had_previous {
        format!("恢复原快捷键 {previous} 失败，当前没有可用快捷键")
    } else {
        "当前没有可用快捷键".to_string()
    };
    format!("注册 {display} 失败: {error}；{status}")
}

#[tauri::command]
pub fn get_overlay_hotkey() -> Result<String, String> {
    #[cfg(windows)]
    {
        Ok(HOTKEY
            .get()
            .and_then(|state| lock(&state.active).clone())
            .unwrap_or_else(|| "未注册".into()))
    }
    #[cfg(not(windows))]
    {
        Ok("未注册".into())
    }
}

/// 探测组合键能否注册：临时注册后立即在同线程注销，不影响热键线程已生效的注册。
/// 返回 Ok(true) = 可用；Ok(false) = 已被占用（ERROR_HOTKEY_ALREADY_REGISTERED）。
#[cfg(windows)]
fn probe_hotkey_available(spec: &HotkeySpec) -> Result<bool, String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
    const PROBE_ID: i32 = 2;
    // SAFETY: 探测注册绑定当前调用线程，且在同一线程内立即注销；
    // 不与热键线程共用 ID，不会干扰已生效的全局热键。
    let registered = unsafe { RegisterHotKey(std::ptr::null_mut(), PROBE_ID, spec.mods, spec.vk) };
    if registered != 0 {
        let unregistered = unsafe { UnregisterHotKey(std::ptr::null_mut(), PROBE_ID) };
        if unregistered == 0 {
            eprintln!(
                "注销探测快捷键失败: {}",
                std::io::Error::last_os_error()
            );
        }
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(1409) {
        return Ok(false);
    }
    Err(format!("检测快捷键占用失败: {err}"))
}

/// 检查组合键是否可用（true = 可用，false = 已被其他程序或本程序占用）。
#[tauri::command]
pub fn check_hotkey_available(hotkey: String) -> Result<bool, String> {
    let (spec, _display) = parse_hotkey(&hotkey)?;
    #[cfg(windows)]
    {
        // 与当前已生效组合相同：属于本程序自己，视为可用
        if let Some(current) = HOTKEY.get().and_then(|state| lock(&state.active).clone()) {
            if let Ok((current_spec, _)) = parse_hotkey(&current) {
                if current_spec.mods == spec.mods && current_spec.vk == spec.vk {
                    return Ok(true);
                }
            }
        }
        probe_hotkey_available(&spec)
    }
    #[cfg(not(windows))]
    {
        Err("当前平台不支持全局快捷键".into())
    }
}

#[tauri::command]
pub fn set_overlay_hotkey(app: tauri::AppHandle, hotkey: String) -> Result<String, String> {
    let (spec, display) = parse_hotkey(&hotkey)?;

    #[cfg(windows)]
    {
        let _ = app;
        let state = HOTKEY.get().ok_or("快捷键服务尚未就绪，请稍后再试")?;
        let _request_guard = lock(&state.request_lock);
        let tid_guard = lock(&state.thread_id);
        let Some(tid) = *tid_guard else {
            return Err("快捷键服务尚未就绪，请稍后再试".into());
        };
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        *lock(&state.desired) = Some(HotkeyRequest {
            spec,
            display,
            reply,
        });
        // SAFETY: 消息队列已建立，tid 在持锁期间不会被热键线程清除。
        let posted = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                tid,
                HOTKEY_CHANGE_MSG,
                0,
                0,
            )
        } != 0;
        if !posted {
            lock(&state.desired).take();
            return Err(format!(
                "通知快捷键线程失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        drop(tid_guard);
        receiver
            .recv()
            .map_err(|_| "快捷键服务已停止".to_string())?
    }

    #[cfg(not(windows))]
    {
        let _ = app;
        let _ = spec;
        Err("当前平台不支持全局快捷键".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_hotkeys() {
        let (spec, display) = parse_hotkey("shift+control+f01").expect("有效快捷键");
        let (same, canonical) = parse_hotkey("Ctrl+Shift+F1").expect("有效快捷键");
        assert_eq!(display, "Ctrl+Shift+F1");
        assert_eq!(display, canonical);
        assert_eq!((spec.mods, spec.vk), (same.mods, same.vk));
    }

    #[test]
    fn rejects_invalid_or_unmodified_character_hotkeys() {
        for value in ["", "A", "Ctrl+F13", "Ctrl+?", "Unknown+F"] {
            assert!(parse_hotkey(value).is_err(), "{value}");
        }
        assert!(parse_hotkey("F12").is_ok());
    }

    #[test]
    fn registration_errors_report_actual_restore_state() {
        assert!(
            registration_error("Ctrl+A", "Alt+F", true, true, "占用")
                .contains("已恢复原快捷键 Alt+F")
        );
        assert!(
            registration_error("Ctrl+A", "Alt+F", true, false, "占用")
                .contains("恢复原快捷键 Alt+F 失败")
        );
        assert!(
            registration_error("Ctrl+A", "Alt+F", false, false, "占用")
                .contains("当前没有可用快捷键")
        );
    }
}

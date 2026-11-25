//! 单实例检测：用命名互斥体判断是否已有实例在运行。
//!
//! 背景：两个进程同时运行会共用同一个 WebView2 用户数据目录
//! （`%LOCALAPPDATA%\<identifier>\EBWebView`），WebView2 不支持多进程共用，
//! 表现为「所有输入框有光标但打不进字」，因此必须保证同时只有一个实例。

use std::sync::OnceLock;

/// 互斥体句柄：故意泄漏，保证进程存活期间一直持有（释放即代表实例退出）
static MUTEX: OnceLock<usize> = OnceLock::new();

/// 初始化检测。返回 true 表示已有另一个实例在运行。
pub fn init() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;

        let name: Vec<u16> = "Local\\endfield-mod-manager-single-instance"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if !h.is_null() {
                // CreateMutexW 在"名字已存在"时也返回有效句柄，靠 GetLastError 区分
                let already = GetLastError() == ERROR_ALREADY_EXISTS;
                let _ = MUTEX.set(h as usize);
                return already;
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 重复启动时的提示：用系统弹窗（此时**绝不**创建 WebView2，
/// 否则第二个 webview 会共用数据目录，把已有实例的输入一起搞坏）。
/// 提示后直接拒绝启动；同时把已在运行的窗口提到前台。
pub fn warn_duplicate_and_exit(title: &str) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MB_ICONINFORMATION, MB_OK, MessageBoxW,
        };

        let text = "「小林若葉Manager」已经在运行。\n\n\
                    同时运行两个实例会导致输入框无法输入文字，因此本次启动已取消。";
        let text_w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let cap_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        MessageBoxW(
            std::ptr::null_mut(),
            text_w.as_ptr(),
            cap_w.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
        // 顺手把已有窗口拉到前台，省得用户自己找
        focus_other_window(title);
    }
    #[cfg(not(windows))]
    {
        let _ = title;
    }
    std::process::exit(0);
}

/// 把"另一个进程"的同名窗口还原并提到前台。
pub fn focus_other_window(title: &str) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{HWND, LPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
            SW_RESTORE, SetForegroundWindow, ShowWindow,
        };

        struct Ctx<'a> {
            title: &'a str,
            my_pid: u32,
            found: HWND,
        }

        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> i32 {
            unsafe {
                let ctx = &mut *(lparam as *mut Ctx);
                if IsWindowVisible(hwnd) == 0 {
                    return 1;
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, &mut pid);
                // 跳过自己进程的窗口
                if pid == ctx.my_pid {
                    return 1;
                }
                let mut buf = [0u16; 512];
                let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                if len > 0 {
                    let s = String::from_utf16_lossy(&buf[..len as usize]);
                    if s == ctx.title {
                        ctx.found = hwnd;
                        return 0; // 找到即停止枚举
                    }
                }
                1
            }
        }

        let mut ctx = Ctx {
            title,
            my_pid: std::process::id(),
            found: std::ptr::null_mut(),
        };
        EnumWindows(Some(cb), &mut ctx as *mut Ctx as LPARAM);
        if !ctx.found.is_null() {
            ShowWindow(ctx.found, SW_RESTORE);
            SetForegroundWindow(ctx.found);
            return true;
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        false
    }
}

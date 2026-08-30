//! 任务栏通知：紧急会话的未读角标 + 一次性任务栏闪烁（桌面端专用，随 `gui` feature 编译）。
//!
//! - Rust 侧后台任务每 2s 扫描事件流目录（复用 [`crate::state::urgent_session_ids`]），
//!   筛出紧急会话（waiting_confirmation / waiting_input / error）；
//! - 未读数 = 紧急会话 − "已读"集合：窗口失焦且有未读时设置任务栏角标
//!   （Windows 用 ITaskbarList3::SetOverlayIcon 叠加红底数字，macOS 用 dock badge），
//!   窗口聚焦时未读数恒为 0 并清掉角标（"打开软件恢复常态"）；
//! - "只闪一次"用独立的"已闪烁"集合实现：某个紧急会话第一次成为未读时调用一次
//!   FlashWindowEx（仅 Windows），之后不再闪；窗口聚焦时两个集合一起清空。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;

/// 后台轮询周期（固定 2s，独立于前端页面的"事件刷新"设置）。
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// 角标计数哨兵：无角标。
const NO_BADGE: usize = usize::MAX;

/// 通知去重状态：已读 / 已闪烁集合 + 窗口聚焦标记 + 上次角标值。
pub struct NotifyState {
    read: Mutex<HashSet<String>>,
    flashed: Mutex<HashSet<String>>,
    focused: AtomicBool,
    /// 上次设置的角标数（[`NO_BADGE`] 表示无）：值未变化时跳过平台调用，
    /// 避免每 2s 重复走 COM / 重建图标。
    last_badge: AtomicUsize,
}

impl Default for NotifyState {
    fn default() -> Self {
        Self {
            read: Mutex::new(HashSet::new()),
            flashed: Mutex::new(HashSet::new()),
            focused: AtomicBool::new(false),
            last_badge: AtomicUsize::new(NO_BADGE),
        }
    }
}

impl NotifyState {
    pub fn set_focused(&self, focused: bool) {
        self.focused.store(focused, Ordering::Relaxed);
    }

    /// 窗口聚焦：清空两个集合，并把当前所有紧急会话记入"已读"。
    pub fn mark_all_read(&self, urgent: Vec<String>) {
        let mut read = self.read.lock().unwrap();
        let mut flashed = self.flashed.lock().unwrap();
        read.clear();
        flashed.clear();
        read.extend(urgent);
    }

    /// 聚焦期间紧急会话直接进入已读（保持"聚焦时未读恒为 0"）。
    fn auto_read(&self, urgent: &HashSet<String>) {
        self.read.lock().unwrap().extend(urgent.iter().cloned());
    }

    /// 未读 = 紧急 − 已读。
    fn unread(&self, urgent: &HashSet<String>) -> Vec<String> {
        let read = self.read.lock().unwrap();
        urgent
            .iter()
            .filter(|id| !read.contains(*id))
            .cloned()
            .collect()
    }

    /// 取出需要首次闪烁的会话（未读且从未闪过），并记入"已闪烁"集合。
    fn take_new_flashes(&self, unread: &[String]) -> Vec<String> {
        let mut flashed = self.flashed.lock().unwrap();
        let fresh: Vec<String> = unread
            .iter()
            .filter(|id| !flashed.contains(*id))
            .cloned()
            .collect();
        for id in &fresh {
            flashed.insert(id.clone());
        }
        fresh
    }
}

/// 轮询一次：计算未读并更新角标 / 触发闪烁。由 gui 的后台定时任务调用。
pub fn poll(app: &tauri::AppHandle, urgent_ids: Vec<String>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<NotifyState>();
    let urgent: HashSet<String> = urgent_ids.into_iter().collect();

    if state.focused.load(Ordering::Relaxed) {
        state.auto_read(&urgent);
        update_badge(&window, &state, None);
        return;
    }

    let unread = state.unread(&urgent);
    if unread.is_empty() {
        update_badge(&window, &state, None);
        return;
    }
    // 新出现的紧急会话触发一次闪烁（同一会话只闪一次）
    if !state.take_new_flashes(&unread).is_empty() {
        flash_window(&window);
    }
    update_badge(&window, &state, Some(unread.len()));
}

/// 窗口聚焦：清空角标，全部紧急会话标记已读，清空"已闪烁"集合。
pub fn on_focus(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        update_badge(&window, &app.state::<NotifyState>(), None);
    }
    app.state::<NotifyState>()
        .mark_all_read(crate::state::urgent_session_ids());
}

/// 设置角标（值未变化时跳过平台调用）。
fn update_badge(window: &tauri::WebviewWindow, state: &NotifyState, count: Option<usize>) {
    let want = count.unwrap_or(NO_BADGE);
    if state.last_badge.swap(want, Ordering::Relaxed) == want {
        return;
    }
    set_badge(window, count);
}

/// 设置任务栏角标：Windows 用 overlay icon（红底数字），macOS 用 dock badge；None 清除。
fn set_badge(window: &tauri::WebviewWindow, count: Option<usize>) {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = window.hwnd() {
            win_taskbar::set_overlay(hwnd.0 as isize, count.map(|c| c as u32));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let label = count.map(|c| if c > 9 { "9+".to_string() } else { c.to_string() });
        let _ = window.set_badge_label(label);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (window, count);
    }
}

/// 闪烁任务栏图标（仅 Windows；每个紧急会话只触发一次）。
fn flash_window(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = window.hwnd() {
            win_taskbar::flash(hwnd.0 as isize);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
    }
}

/// Win32 任务栏适配：overlay 角标（ITaskbarList3）+ 闪烁（FlashWindowEx）。
#[cfg(windows)]
mod win_taskbar {
    use std::sync::Mutex;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{COLORREF, HWND, RECT, RPC_E_CHANGED_MODE};
    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
        DeleteDC, DeleteObject, DrawTextW, FillRect, GetDC, ReleaseDC, SelectObject, SetBkMode,
        SetTextColor, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DEFAULT_QUALITY,
        DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_BOLD, HGDIOBJ, OUT_DEFAULT_PRECIS, TRANSPARENT,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, DestroyIcon, FlashWindowEx, GetSystemMetrics, FLASHWINFO, FLASHW_ALL,
        HICON, ICONINFO, SM_CXSMICON,
    };

    /// 上一次成功设置的 overlay 图标（替换 / 清除时销毁，避免泄漏）。
    static LAST_HICON: Mutex<Option<usize>> = Mutex::new(None);

    /// 设置 overlay 角标（count 为 None 时清除）。
    pub fn set_overlay(hwnd: isize, count: Option<u32>) {
        unsafe {
            let new_icon = count.map(|c| make_badge_icon(c));
            let set = with_com(|| {
                let taskbar: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_ALL)?;
                taskbar.SetOverlayIcon(HWND(hwnd as _), new_icon.unwrap_or_default(), PCWSTR::null())
            });
            if set.is_ok() {
                destroy_last_icon();
                if let Some(h) = new_icon {
                    *LAST_HICON.lock().unwrap() = Some(h.0 as usize);
                }
            } else if let Some(h) = new_icon {
                let _ = DestroyIcon(h);
            }
        }
    }

    /// 闪烁任务栏按钮（标题栏 + 任务栏闪几次即停，不持续闪烁打扰）。
    pub fn flash(hwnd: isize) {
        unsafe {
            let info = FLASHWINFO {
                cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                hwnd: HWND(hwnd as _),
                dwFlags: FLASHW_ALL,
                uCount: 4,
                dwTimeout: 0,
            };
            let _ = FlashWindowEx(&info);
        }
    }

    /// 在当前线程初始化 COM 后执行 f（后台线程默认未初始化 COM）。
    /// 线程已按其他模式初始化（RPC_E_CHANGED_MODE）时直接复用。
    unsafe fn with_com<T>(
        f: impl FnOnce() -> windows::core::Result<T>,
    ) -> windows::core::Result<T> {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let need_uninit = if hr.is_ok() {
            true
        } else if hr == RPC_E_CHANGED_MODE {
            false
        } else {
            return Err(windows::core::Error::from_hresult(hr));
        };
        let out = f();
        if need_uninit {
            CoUninitialize();
        }
        out
    }

    unsafe fn destroy_last_icon() {
        let mut last = LAST_HICON.lock().unwrap();
        if let Some(v) = last.take() {
            let _ = DestroyIcon(HICON(v as _));
        }
    }

    /// 生成红底白字数字角标图标（尺寸取系统小图标尺寸，>9 显示 "9+"）。
    unsafe fn make_badge_icon(count: u32) -> HICON {
        let size = GetSystemMetrics(SM_CXSMICON).clamp(16, 48);
        let wide = count > 9;
        let mut text: Vec<u16> = if wide {
            "9+".encode_utf16().collect()
        } else {
            count.to_string().encode_utf16().collect()
        };

        let hdc_screen = GetDC(None);
        let hdc = CreateCompatibleDC(Some(hdc_screen));
        let hbm_color = CreateCompatibleBitmap(hdc_screen, size, size);
        let hbm_mask = CreateBitmap(size, size, 1, 1, None);
        let old_bmp = SelectObject(hdc, HGDIOBJ(hbm_color.0));

        // 红底（COLORREF 布局为 0x00BBGGRR）
        let brush = CreateSolidBrush(COLORREF(0x0000FF));
        let mut rc = RECT { left: 0, top: 0, right: size, bottom: size };
        FillRect(hdc, &rc, brush);

        // 白色粗体数字居中
        let font_h = if wide { size * 9 / 16 } else { size * 11 / 16 };
        let mut face: Vec<u16> = "Segoe UI".encode_utf16().collect();
        face.push(0);
        let font = CreateFontW(
            font_h, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, DEFAULT_PITCH.0 as u32,
            windows::core::PWSTR(face.as_mut_ptr()),
        );
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(0xFFFFFF));
        DrawTextW(hdc, &mut text, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let hicon = CreateIconIndirect(&info).unwrap_or_default();

        // 清理 GDI 资源（CreateIconIndirect 会拷贝位图，原位图可安全删除）
        SelectObject(hdc, old_bmp);
        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteDC(hdc);
        ReleaseDC(None, hdc_screen);
        hicon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unread_and_flash_dedup() {
        let s = NotifyState::default();
        let urgent = ids(&["a", "b"]);
        assert_eq!(s.unread(&urgent).len(), 2, "初始全部未读");

        let unread = s.unread(&urgent);
        assert_eq!(s.take_new_flashes(&unread).len(), 2, "首次全部触发闪烁");
        let unread = s.unread(&urgent);
        assert!(s.take_new_flashes(&unread).is_empty(), "同一会话只闪一次");

        // 窗口聚焦：全部标记已读、清空已闪烁集合
        s.mark_all_read(vec!["a".into(), "b".into()]);
        assert!(s.unread(&urgent).is_empty(), "聚焦后未读清零");

        // 新出现的紧急会话：未读且未闪过 → 再次触发一次闪烁
        let mut urgent2 = urgent.clone();
        urgent2.insert("c".into());
        let unread = s.unread(&urgent2);
        assert_eq!(unread, vec!["c".to_string()]);
        assert_eq!(s.take_new_flashes(&unread).len(), 1);
    }

    #[test]
    fn focused_auto_read() {
        let s = NotifyState::default();
        s.set_focused(true);
        let urgent = ids(&["a"]);
        // 聚焦期间紧急会话直接进入已读
        s.auto_read(&urgent);
        assert!(s.unread(&urgent).is_empty());
    }
}

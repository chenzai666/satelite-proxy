//! Native title-bar icons (Windows).
//!
//! Tauri's default window icon extracts a single RGBA frame from `icon.ico`
//! and tao turns it into one big `CreateIcon` HICON set via
//! `WM_SETICON(ICON_SMALL)` — the title bar then GDI-downscales that frame to
//! its 16–24px slot, which reads as mush no matter how well the .ico's small
//! entries are tuned (the neon saturn tile made this obvious).
//!
//! So the title bar gets its own artwork (chosen by the user: the mint
//! satellite badge, same design as `tray-icon-running.png`): per-DPI-size
//! renders are embedded as PNGs (`scripts/generate-tray-icons.py`, `titlebar-*`)
//! and `CreateIcon`d at their native size — the title bar never rescales.
//! ICON_BIG (taskbar / Alt-Tab) stays the app icon, loaded per DPI from the
//! exe's icon resource group (tauri-build embeds the bundle .ico under
//! resource id 32512).
//!
//! Re-applied on window (re)creation and on ScaleFactorChanged /
//! Focused(true) — early in startup `GetDpiForWindow` can briefly report the
//! primary monitor's DPI, so a (hwnd, dpi) memo makes the refresh hooks
//! cheap no-ops once the icon is correct. WM_SETICON does not copy handles;
//! per-apply LoadImage handles are kept alive until replaced, cached badge
//! handles live forever (single-window app, a handful of 16–48px bitmaps).

#[cfg(windows)]
mod native {
    use std::sync::Mutex;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIcon, DestroyIcon, GetSystemMetrics, LoadImageW, SendMessageW, HICON, ICON_BIG,
        ICON_SMALL, ICON_SMALL2, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
    };

    /// tauri-build embeds the bundle icon group under this resource id.
    const APP_ICON_RESOURCE_ID: usize = 32512;

    /// Title-bar badge renders (mint satellite on dark tile), one per common
    /// small-icon DPI size — regenerated via scripts/generate-tray-icons.py.
    const BADGE_PNGS: &[(u32, &[u8])] = &[
        (16, include_bytes!("../icons/tray/titlebar-16.png")),
        (20, include_bytes!("../icons/tray/titlebar-20.png")),
        (24, include_bytes!("../icons/tray/titlebar-24.png")),
        (28, include_bytes!("../icons/tray/titlebar-28.png")),
        (32, include_bytes!("../icons/tray/titlebar-32.png")),
        (40, include_bytes!("../icons/tray/titlebar-40.png")),
        (48, include_bytes!("../icons/tray/titlebar-48.png")),
    ];

    /// Created badge HICONs, cached per embedded size.
    static BADGE_CACHE: Mutex<Vec<(u32, isize)>> = Mutex::new(Vec::new());
    /// LoadImage handles owned by the current WM_SETICON batch.
    static LIVE_APP_ICONS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
    /// (hwnd, dpi) the current icon batch was applied for.
    static APPLIED_FOR: Mutex<(usize, u32)> = Mutex::new((0, 0));

    fn badge_icon(size: i32) -> Option<isize> {
        if size <= 0 {
            return None;
        }
        let want = size as u32;
        if let Ok(cache) = BADGE_CACHE.lock() {
            if let Some(&(_, handle)) = cache.iter().find(|&&(s, _)| s == want) {
                return Some(handle);
            }
        }
        // Nearest embedded render; the title bar cell rescales the few-px
        // gap, if any, which is invisible.
        let &(render_size, bytes) = BADGE_PNGS.iter().min_by_key(|&&(s, _)| s.abs_diff(want))?;
        let image = tauri::image::Image::from_bytes(bytes).ok()?;
        let (w, h) = (image.width() as i32, image.height() as i32);
        let rgba = image.rgba();
        if rgba.len() != (w * h * 4) as usize {
            return None;
        }
        // Same layout tao uses: AND mask = inverted alpha (one byte per
        // pixel), color plane as BGRA.
        let mut color = Vec::with_capacity(rgba.len());
        let mut mask = Vec::with_capacity(rgba.len());
        for px in rgba.chunks_exact(4) {
            mask.push(px[3].wrapping_sub(u8::MAX));
            color.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        let handle = unsafe {
            CreateIcon(None, w, h, 1, 32, mask.as_ptr(), color.as_ptr())
                .ok()?
                .0 as isize
        };
        if let Ok(mut cache) = BADGE_CACHE.lock() {
            cache.push((render_size, handle));
        }
        Some(handle)
    }

    fn apply_to_hwnd(hwnd: HWND) {
        let raw = hwnd.0 as usize;
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        {
            // Cheap memo: refresh hooks (Focused) hit this constantly.
            if let Ok(applied) = APPLIED_FOR.lock() {
                if *applied == (raw, dpi) {
                    return;
                }
            }
        }
        let mut fresh_app_icons = Vec::new();
        // Title-bar slots: the dedicated badge at the per-DPI small size.
        if let Some(handle) = badge_icon(small_icon_size(dpi)) {
            for slot in [ICON_SMALL, ICON_SMALL2] {
                unsafe {
                    // WM_SETICON: wParam = icon slot, lParam = HICON handle.
                    SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(slot as usize)),
                        Some(LPARAM(handle)),
                    );
                }
            }
        }
        // Taskbar / Alt-Tab slot: the app icon from the exe's resource group.
        let big_metric = if dpi > 0 {
            unsafe { GetSystemMetricsForDpi(SM_CXICON, dpi) }
        } else {
            unsafe { GetSystemMetrics(SM_CXICON) }
        };
        if let Some(handle) = load_app_icon_for(big_metric) {
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(handle)),
                );
            }
            fresh_app_icons.push(handle);
        }
        crate::app_log::debug(
            "window_icon",
            format!("applied hwnd={raw:x} dpi={dpi} big={big_metric}px"),
        );
        if let Ok(mut applied) = APPLIED_FOR.lock() {
            *applied = (raw, dpi);
        }
        if let Ok(mut live) = LIVE_APP_ICONS.lock() {
            for old in live.drain(..) {
                unsafe {
                    let _ = DestroyIcon(HICON(old as *mut _));
                }
            }
            *live = fresh_app_icons;
        }
    }

    fn small_icon_size(dpi: u32) -> i32 {
        if dpi > 0 {
            unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }
        } else {
            unsafe { GetSystemMetrics(SM_CXSMICON) }
        }
    }

    fn load_app_icon_for(size: i32) -> Option<isize> {
        if size <= 0 {
            return None;
        }
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).ok()?;
            let handle = LoadImageW(
                Some(HINSTANCE(instance.0)),
                PCWSTR(APP_ICON_RESOURCE_ID as *const u16),
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            )
            .ok()?;
            Some(handle.0 as isize)
        }
    }

    pub fn apply_to<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
        if let Ok(hwnd) = window.hwnd() {
            apply_to_hwnd(hwnd);
        }
    }

    pub fn apply_to_window<R: tauri::Runtime>(window: &tauri::Window<R>) {
        if let Ok(hwnd) = window.hwnd() {
            apply_to_hwnd(hwnd);
        }
    }
}

#[cfg(windows)]
pub use native::{apply_to, apply_to_window};

#[cfg(not(windows))]
pub fn apply_to<R: tauri::Runtime>(_: &tauri::WebviewWindow<R>) {}

#[cfg(not(windows))]
pub fn apply_to_window<R: tauri::Runtime>(_: &tauri::Window<R>) {}

//! Places a native video window underneath the (transparent) WebView2, so
//! libVLC draws the video and DWM composites the HTML UI on top.

use std::ffi::c_void;
use std::sync::Once;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, HWND_BOTTOM, RegisterClassW, SWP_NOACTIVATE, SetWindowPos,
    WM_SIZE, WNDCLASSW, WS_CHILD, WS_DISABLED, WS_VISIBLE,
};

/// Keeps the video window the size of the main window's client area.
unsafe extern "system" fn parent_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, video: usize) -> LRESULT {
    unsafe {
        if msg == WM_SIZE {
            fit(hwnd, video as HWND);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

unsafe fn fit(parent: HWND, video: HWND) {
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(parent, &mut rect);
        SetWindowPos(video, HWND_BOTTOM, 0, 0, rect.right - rect.left, rect.bottom - rect.top, SWP_NOACTIVATE);
    }
}

/// Creates the video window below every other child of `parent` (the main
/// window, whose WebView2 sits on top) and returns it as an `HWND`. Must run
/// on the main thread. The window lives as long as its parent.
pub fn attach(parent: *mut c_void) -> Result<*mut c_void, String> {
    let parent = parent as HWND;
    let class_name: Vec<u16> = "ReVLCVideo\0".encode_utf16().collect();
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&class);
        });
        // Disabled, so clicks that reach it (none should: the web view covers
        // it) never take focus from the web view.
        let video = CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_DISABLED,
            0,
            0,
            0,
            0,
            parent,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if video.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        fit(parent, video);
        if SetWindowSubclass(parent, Some(parent_proc), 1, video as usize) == 0 {
            return Err("could not follow the window's size".into());
        }
        Ok(video)
    }
}

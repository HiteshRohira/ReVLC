//! System information on Windows: installed fonts and preferred languages.

use windows_sys::Win32::Foundation::LPARAM;
use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows_sys::Win32::Graphics::Gdi::{DEFAULT_CHARSET, EnumFontFamiliesExW, GetDC, LOGFONTW, ReleaseDC, TEXTMETRICW};

/// Installed font families, for the subtitle font picker.
pub fn font_families() -> Vec<String> {
    unsafe extern "system" fn add(font: *const LOGFONTW, _: *const TEXTMETRICW, _: u32, fonts: LPARAM) -> i32 {
        unsafe {
            let name = &(*font).lfFaceName;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            let name = String::from_utf16_lossy(&name[..len]);
            // "@" families are the vertical variants of CJK fonts.
            if !name.is_empty() && !name.starts_with('@') {
                (*(fonts as *mut Vec<String>)).push(name);
            }
        }
        1
    }
    let mut fonts: Vec<String> = Vec::new();
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let mut query: LOGFONTW = std::mem::zeroed();
        query.lfCharSet = DEFAULT_CHARSET as u8;
        EnumFontFamiliesExW(dc, &query, Some(add), &mut fonts as *mut Vec<String> as LPARAM, 0);
        ReleaseDC(std::ptr::null_mut(), dc);
    }
    fonts.sort_by_key(|f| f.to_lowercase());
    fonts.dedup();
    fonts
}

/// The user's preferred languages as two-letter codes, most preferred first.
pub fn preferred_languages() -> Vec<String> {
    let mut count = 0u32;
    let mut len = 0u32;
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, std::ptr::null_mut(), &mut len) == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buf.as_mut_ptr(), &mut len) == 0 {
            return Vec::new();
        }
        // A list of names like "en-GB", each ending in a NUL.
        let mut langs: Vec<String> = Vec::new();
        for name in buf.split(|&c| c == 0).filter(|n| !n.is_empty()) {
            let code = String::from_utf16_lossy(name).split('-').next().unwrap_or_default().to_lowercase();
            if !code.is_empty() && !langs.contains(&code) {
                langs.push(code);
            }
        }
        langs
    }
}

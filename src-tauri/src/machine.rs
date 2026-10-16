//! What the app runs on, for analytics: OS version, CPU, memory, model.

#[derive(Default)]
pub struct Machine {
    /// PostHog's names: "Mac OS X", "Windows".
    pub os: &'static str,
    /// "15.2" on macOS; "10" or "11" on Windows.
    pub os_version: String,
    /// The build: "24C101" on macOS, "22631.4460" on Windows.
    pub os_build: String,
    pub cpu: String,
    pub ram_gb: u64,
    /// "Mac14,2", or the PC maker's product name.
    pub model: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn info() -> Machine {
    use std::ffi::CString;

    fn sysctl(name: &str) -> Option<Vec<u8>> {
        let name = CString::new(name).ok()?;
        let mut len = 0usize;
        unsafe {
            if libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
                return None;
            }
            let mut buf = vec![0u8; len];
            if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) != 0 {
                return None;
            }
            buf.truncate(len);
            Some(buf)
        }
    }
    let text = |name: &str| {
        sysctl(name).map(|b| String::from_utf8_lossy(&b).trim_end_matches('\0').trim().to_string()).unwrap_or_default()
    };
    let memory = sysctl("hw.memsize").and_then(|b| b.try_into().ok()).map_or(0, u64::from_ne_bytes);
    Machine {
        os: "Mac OS X",
        os_version: text("kern.osproductversion"),
        os_build: text("kern.osversion"),
        cpu: text("machdep.cpu.brand_string"),
        ram_gb: gigabytes(memory),
        model: Some(text("hw.model")).filter(|m| !m.is_empty()),
    }
}

#[cfg(windows)]
pub fn info() -> Machine {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn text(key: &str, value: &str) -> String {
        let (key, value) = (wide(key), wide(value));
        let mut buf = [0u16; 256];
        let mut len = std::mem::size_of_val(&buf) as u32;
        let ok = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
        } == 0;
        if !ok {
            return String::new();
        }
        let chars = (len as usize / 2).min(buf.len());
        String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').trim().to_string()
    }
    fn dword(key: &str, value: &str) -> Option<u32> {
        let (key, value) = (wide(key), wide(value));
        let mut out = 0u32;
        let mut len = 4u32;
        let ok = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut out as *mut u32).cast(), &mut len)
        } == 0;
        ok.then_some(out)
    }

    const VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = text(VERSION, "CurrentBuildNumber");
    // Windows 11 still calls itself 10 in the registry; its builds start at 22000.
    let os_version = match build.parse::<u32>() {
        Ok(n) if n >= 22000 => "11",
        Ok(_) => "10",
        Err(_) => "",
    };
    let os_build = match dword(VERSION, "UBR") {
        Some(ubr) if !build.is_empty() => format!("{build}.{ubr}"),
        _ => build,
    };
    let mut memory: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    memory.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ram = if unsafe { GlobalMemoryStatusEx(&mut memory) } != 0 { memory.ullTotalPhys } else { 0 };
    let model = text(r"HARDWARE\DESCRIPTION\System\BIOS", "SystemProductName");
    Machine {
        os: "Windows",
        os_version: os_version.into(),
        os_build,
        cpu: text(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString"),
        ram_gb: gigabytes(ram),
        model: Some(model).filter(|m| !m.is_empty() && m != "System Product Name"),
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn info() -> Machine {
    Machine { os: "Linux", ..Default::default() }
}

/// Rounded to whole gigabytes (the OS keeps a little back, so 16 GB machines report a bit less).
fn gigabytes(bytes: u64) -> u64 {
    (bytes as f64 / (1u64 << 30) as f64).round() as u64
}
